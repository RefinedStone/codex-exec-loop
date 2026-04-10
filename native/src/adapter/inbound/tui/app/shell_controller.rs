use super::planning_draft_editor_ui::{
    PlanningDraftEditorCloseRequest, PlanningDraftEditorCloseRisk,
};
use super::session_browser::{
    SessionBrowserSelection, SessionBrowserView, build_session_browser_view,
};
use super::*;
use crate::application::service::planning_prompt_service::PlanningPromptContextLoadResult;
use crate::application::service::session_service::project_recent_sessions;

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
            (PromptOrigin::AutoFollow(_), ShellActionAvailability::Pending) => {
                "auto follow-up paused while startup checks are still running".to_string()
            }
            (PromptOrigin::AutoFollow(_), ShellActionAvailability::Blocked) => {
                "auto follow-up paused because startup diagnostics need attention".to_string()
            }
            (PromptOrigin::PlanningRepair(_), ShellActionAvailability::Pending) => {
                "planning repair paused while startup checks are still running".to_string()
            }
            (PromptOrigin::PlanningRepair(_), ShellActionAvailability::Blocked) => {
                "planning repair paused because startup diagnostics need attention".to_string()
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
        self.refresh_ready_conversation_planning_prompt_context();
    }

    pub(super) fn reload_followup_templates(&mut self) {
        let workspace_directory = match &self.conversation_state {
            ConversationState::Ready(conversation) => conversation.cwd.clone(),
            ConversationState::Loading | ConversationState::Failed(_) => return,
        };

        self.dispatch_followup_controls(FollowupControlEvent::TemplateCatalogReloaded {
            reload_result: self
                .followup_template_service
                .reload_catalog(&workspace_directory),
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
            max_auto_turns: self.current_max_auto_turns_value().to_string(),
        });
        self.dispatch_shell_chrome(ShellChromeEvent::FollowupTemplatesOverlayShown);
    }

    pub(super) fn show_planning_init_overlay(&mut self) {
        self.planning_init_overlay_ui_state.reset();
        self.planning_draft_editor_ui_state.reset();
        self.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
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
        if self.shell_overlay == ShellOverlay::PlanningInit {
            self.planning_init_overlay_ui_state.reset();
            self.planning_draft_editor_ui_state.reset();
        }
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
            InlineShellCommand::PlanningInit => self.show_planning_init_overlay(),
            InlineShellCommand::NewDraft => self.open_new_conversation_shell(),
            InlineShellCommand::TranscriptTopLegacy => {}
            InlineShellCommand::TranscriptTailLegacy => {}
            InlineShellCommand::Help => {}
        }

        if let Some(status_text) = command.execution_status() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: status_text.to_string(),
            });
        }
        self.clear_input_buffer();
    }

    pub(super) fn open_planning_manual_editor(&mut self) {
        let workspace_directory = self.current_workspace_directory();
        let status_text = match self
            .planning_init_service
            .stage_manual_editor_session(&workspace_directory)
        {
            Ok(session) => {
                let draft_name = session.draft_name.clone();
                let validation_ok = session.validation_report.is_valid();
                self.planning_draft_editor_ui_state.open_session(session);
                self.planning_init_overlay_ui_state.open_manual_editor();
                format!(
                    "planning draft editor ready / draft: {draft_name} / validation: {}",
                    if validation_ok {
                        "ok"
                    } else {
                        "needs attention"
                    }
                )
            }
            Err(error) => format!("planning init failed: {error}"),
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    pub(super) fn save_planning_manual_editor(&mut self) {
        let Some(draft_name) = self
            .planning_draft_editor_ui_state
            .draft_name()
            .map(str::to_string)
        else {
            return;
        };
        self.planning_draft_editor_ui_state
            .clear_close_confirmation();
        let workspace_directory = self.current_workspace_directory();
        let editable_files = self.planning_draft_editor_ui_state.collect_editable_files();
        let status_text = match self.planning_init_service.save_draft_editor_files(
            &workspace_directory,
            &draft_name,
            &editable_files,
        ) {
            Ok(result) => {
                let validation_ok = result.validation_report.is_valid();
                self.planning_draft_editor_ui_state
                    .apply_save_result(result.validation_report.clone());
                format!(
                    "planning draft saved / draft: {} / validation: {}",
                    result.draft_name,
                    if validation_ok {
                        "ok"
                    } else {
                        "needs attention"
                    }
                )
            }
            Err(error) => format!("planning draft save failed: {error}"),
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    pub(super) fn promote_planning_manual_editor(&mut self) {
        let Some(draft_name) = self
            .planning_draft_editor_ui_state
            .draft_name()
            .map(str::to_string)
        else {
            return;
        };
        self.planning_draft_editor_ui_state
            .clear_close_confirmation();
        let workspace_directory = self.current_workspace_directory();
        let editable_files = self.planning_draft_editor_ui_state.collect_editable_files();
        let status_text = match self.planning_init_service.promote_draft_editor_files(
            &workspace_directory,
            &draft_name,
            &editable_files,
        ) {
            Ok(result) => {
                let validation_ok = result.validation_report.is_valid();
                self.planning_draft_editor_ui_state
                    .apply_save_result(result.validation_report.clone());
                if result.promoted_file_count == 0 {
                    format!(
                        "planning draft promote blocked / draft: {} / validation: {}",
                        result.draft_name,
                        if validation_ok {
                            "ok"
                        } else {
                            "needs attention"
                        }
                    )
                } else {
                    self.close_shell_overlay();
                    self.refresh_ready_conversation_planning_prompt_context();
                    format!(
                        "planning draft promoted / draft: {} / files: {} / planning context refreshed",
                        result.draft_name, result.promoted_file_count
                    )
                }
            }
            Err(error) => format!("planning draft promote failed: {error}"),
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    fn request_close_planning_manual_editor(&mut self) {
        match self.planning_draft_editor_ui_state.request_close() {
            PlanningDraftEditorCloseRequest::CloseImmediately => self.close_shell_overlay(),
            PlanningDraftEditorCloseRequest::ConfirmationRequired(risk) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: planning_manual_editor_close_warning_status(risk),
                });
            }
            PlanningDraftEditorCloseRequest::Confirmed(risk) => {
                self.close_planning_manual_editor_after_confirmation(risk);
            }
        }
    }

    fn close_planning_manual_editor_after_confirmation(
        &mut self,
        risk: PlanningDraftEditorCloseRisk,
    ) {
        self.close_shell_overlay();
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: planning_manual_editor_closed_status(risk),
        });
    }

    fn handle_planning_manual_editor_close_confirmation_key(
        &mut self,
        key: event::KeyEvent,
    ) -> bool {
        if !self
            .planning_draft_editor_ui_state
            .is_close_confirmation_pending()
        {
            return false;
        }

        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => {
                let Some(risk) = self.planning_draft_editor_ui_state.pending_close_risk() else {
                    return false;
                };
                self.close_planning_manual_editor_after_confirmation(risk);
                true
            }
            KeyCode::Char('n') | KeyCode::Char('N')
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.planning_draft_editor_ui_state
                    .clear_close_confirmation();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: "planning draft editor close canceled; keep editing".to_string(),
                });
                true
            }
            _ => {
                self.planning_draft_editor_ui_state
                    .clear_close_confirmation();
                false
            }
        }
    }

    pub(super) fn stage_simple_mode_planning_init_draft(&mut self) {
        let workspace_directory = self.current_workspace_directory();
        let status_text = match self
            .planning_init_service
            .stage_simple_editor_session(&workspace_directory)
        {
            Ok(session) => {
                let draft_name = session.draft_name.clone();
                let validation_ok = session.validation_report.is_valid();
                self.planning_draft_editor_ui_state.open_session(session);
                self.planning_init_overlay_ui_state.open_manual_editor();
                format!(
                    "planning simple draft ready / draft: {draft_name} / validation: {}",
                    if validation_ok {
                        "ok"
                    } else {
                        "needs attention"
                    }
                )
            }
            Err(error) => format!("planning init failed: {error}"),
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    pub(super) fn current_session(&self) -> Option<&SessionSummary> {
        self.current_session_browser_view()
            .and_then(|browser_view| browser_view.selected_session())
    }

    pub(super) fn open_conversation_shell(&mut self) {
        self.dispatch_conversation_intent(ConversationIntentEvent::SessionOpenRequested {
            session: self.current_session().cloned(),
        });
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        let Some(next_selection) = self
            .current_session_browser_view()
            .map(|browser_view| browser_view.selection_after_delta(delta))
        else {
            return;
        };

        self.apply_session_browser_selection(next_selection);
    }

    pub(super) fn jump_to_first_session(&mut self) {
        self.session_overlay_ui_state.jump_to_first_page();
        let next_selection = self
            .current_session_browser_view()
            .map(|browser_view| browser_view.first_selection())
            .unwrap_or(SessionBrowserSelection {
                index: 0,
                session_id: None,
            });
        self.apply_session_browser_selection(next_selection);
    }

    pub(super) fn jump_to_last_session(&mut self) {
        let total_pages = self
            .current_session_browser_view()
            .map(|browser_view| browser_view.projection.total_pages)
            .unwrap_or(0);
        self.session_overlay_ui_state.jump_to_last_page(total_pages);
        let next_selection = self
            .current_session_browser_view()
            .map(|browser_view| browser_view.last_selection())
            .unwrap_or(SessionBrowserSelection {
                index: 0,
                session_id: None,
            });
        self.apply_session_browser_selection(next_selection);
    }

    pub(super) fn clear_session_browser_state(&mut self) {
        self.selected_session_index = 0;
        self.session_overlay_ui_state.clear_browser_state();
        self.sync_session_browser_selection();
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

    pub(super) fn delete_previous_input_word(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::PreviousWordDeleted);
    }

    pub(super) fn clear_prompt_input(&mut self) {
        self.clear_input_buffer();
    }

    pub(super) fn toggle_auto_followup(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::AutoFollowToggled);
    }

    pub(super) fn current_max_auto_turns_value(&self) -> usize {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.auto_follow_state.max_auto_turns_value()
            }
            ConversationState::Loading | ConversationState::Failed(_) => {
                DEFAULT_AUTO_FOLLOW_MAX_TURNS
            }
        }
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

    pub(super) fn is_max_auto_turns_editing(&self) -> bool {
        self.followup_overlay_ui_state
            .max_auto_turns_editor
            .is_editing
    }

    pub(super) fn is_stop_keyword_editing(&self) -> bool {
        self.followup_overlay_ui_state
            .stop_keyword_editor
            .is_editing
    }

    pub(super) fn start_max_auto_turns_edit(&mut self) {
        if !matches!(self.conversation_state, ConversationState::Ready(_)) {
            return;
        }

        if self.shell_overlay != ShellOverlay::FollowupTemplates {
            self.show_followup_template_overlay();
        }

        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsEditStarted {
            current_value: self.current_max_auto_turns_value().to_string(),
        });
    }

    pub(super) fn save_max_auto_turns_edit(&mut self) {
        if !self.is_max_auto_turns_editing() {
            return;
        }

        self.dispatch_followup_controls(FollowupControlEvent::MaxAutoTurnsUpdated {
            value: self
                .followup_overlay_ui_state
                .max_auto_turns_editor
                .buffer
                .clone(),
        });
    }

    pub(super) fn cancel_max_auto_turns_edit(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsEditCanceled {
            current_value: self.current_max_auto_turns_value().to_string(),
        });
    }

    pub(super) fn push_max_auto_turns_character(&mut self, character: char) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsCharacterTyped {
            character,
        });
    }

    pub(super) fn pop_max_auto_turns_character(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsBackspacePressed);
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

    pub(super) fn load_planning_prompt_context(
        &self,
        workspace_directory: &str,
    ) -> PlanningPromptContextLoadResult {
        self.planning_prompt_service
            .load_prompt_context(workspace_directory)
            .unwrap_or_else(|error| planning_prompt_context_load_failed(error.to_string()))
    }

    pub(super) fn refresh_ready_conversation_planning_prompt_context(&mut self) {
        let Some(mut conversation) = self.take_ready_conversation_state() else {
            return;
        };
        let workspace_directory = conversation.cwd.clone();
        conversation.replace_planning_prompt_context(
            self.load_planning_prompt_context(&workspace_directory),
        );
        self.conversation_state = ConversationState::Ready(conversation);
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

    pub(super) fn handle_max_auto_turns_editor_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay != ShellOverlay::FollowupTemplates
            || !self.is_max_auto_turns_editing()
        {
            return false;
        }

        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => self.save_max_auto_turns_edit(),
            KeyCode::Esc => self.cancel_max_auto_turns_edit(),
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.cancel_max_auto_turns_edit()
            }
            KeyCode::Backspace => self.pop_max_auto_turns_character(),
            KeyCode::Char(character)
                if (key.modifiers == KeyModifiers::NONE
                    || key.modifiers == KeyModifiers::SHIFT)
                    && character.is_ascii_digit() =>
            {
                self.push_max_auto_turns_character(character);
            }
            _ => {}
        }

        true
    }

    fn current_session_browser_view(&self) -> Option<SessionBrowserView<'_>> {
        let current_workspace_directory = self.current_workspace_directory();
        match &self.session_state {
            SessionState::Ready(recent_sessions) => Some(build_session_browser_view(
                recent_sessions,
                self.session_overlay_ui_state.browser_state(),
                Some(current_workspace_directory.as_str()),
                self.session_overlay_ui_state.selected_session_id(),
                self.selected_session_index,
            )),
            _ => None,
        }
    }

    fn apply_session_browser_selection(&mut self, selection: SessionBrowserSelection) {
        self.selected_session_index = selection.index;
        self.session_overlay_ui_state
            .set_selected_session_id(selection.session_id);
    }

    fn sync_session_browser_selection(&mut self) {
        let (selected_session_index, selected_session_id) =
            match self.current_session_browser_view() {
                Some(browser_view) => (
                    browser_view.selected_index.unwrap_or(0),
                    browser_view
                        .selected_session()
                        .map(|session| session.id.clone()),
                ),
                None => (0, None),
            };

        self.selected_session_index = selected_session_index;
        self.session_overlay_ui_state
            .set_selected_session_id(selected_session_id);
    }

    pub(super) fn is_session_search_query_editing(&self) -> bool {
        self.session_overlay_ui_state.is_search_query_editing()
    }

    pub(super) fn start_session_search_query_edit(&mut self) {
        if self.shell_overlay != ShellOverlay::Sessions {
            return;
        }

        self.session_overlay_ui_state.start_search_query_edit();
    }

    pub(super) fn save_session_search_query_edit(&mut self) {
        if !self.is_session_search_query_editing() {
            return;
        }

        self.session_overlay_ui_state.save_search_query_edit();
        self.sync_session_browser_selection();
    }

    pub(super) fn cancel_session_search_query_edit(&mut self) {
        if !self.is_session_search_query_editing() {
            return;
        }

        self.session_overlay_ui_state.cancel_search_query_edit();
    }

    pub(super) fn push_session_search_query_character(&mut self, character: char) {
        self.session_overlay_ui_state
            .push_search_query_character(character);
    }

    pub(super) fn pop_session_search_query_character(&mut self) {
        self.session_overlay_ui_state.pop_search_query_character();
    }

    pub(super) fn cycle_session_project_filter(&mut self, delta: isize) {
        let SessionState::Ready(recent_sessions) = &self.session_state else {
            return;
        };

        let projection = project_recent_sessions(
            recent_sessions,
            self.session_overlay_ui_state.browser_state(),
            Some(self.current_workspace_directory().as_str()),
        );
        let Some(next_filter) = projection.cycled_project_filter(delta) else {
            return;
        };

        self.session_overlay_ui_state
            .set_project_filter(next_filter);
        self.sync_session_browser_selection();
    }

    pub(super) fn move_session_page(&mut self, delta: isize) {
        let SessionState::Ready(recent_sessions) = &self.session_state else {
            return;
        };

        let total_pages = project_recent_sessions(
            recent_sessions,
            self.session_overlay_ui_state.browser_state(),
            Some(self.current_workspace_directory().as_str()),
        )
        .total_pages;
        self.session_overlay_ui_state.move_page(delta, total_pages);
        self.sync_session_browser_selection();
    }

    pub(super) fn handle_session_search_query_editor_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay != ShellOverlay::Sessions || !self.is_session_search_query_editing() {
            return false;
        }

        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => self.save_session_search_query_edit(),
            KeyCode::Esc => self.cancel_session_search_query_edit(),
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.cancel_session_search_query_edit()
            }
            KeyCode::Backspace => self.pop_session_search_query_character(),
            KeyCode::Char(character)
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.push_session_search_query_character(character);
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

        if self.handle_max_auto_turns_editor_key(key) {
            return true;
        }

        if self.handle_stop_keyword_editor_key(key) {
            return true;
        }

        if self.handle_session_search_query_editor_key(key) {
            return true;
        }

        if key.code == KeyCode::Esc
            || (key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c'))
        {
            let closing_planning_manual_editor = self.shell_overlay == ShellOverlay::PlanningInit
                && self.planning_init_overlay_ui_state.step()
                    == PlanningInitOverlayStep::ManualEditor;
            if closing_planning_manual_editor {
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
                KeyCode::Char('r') if key.modifiers.is_empty() => self.reload_followup_templates(),
                KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => {
                    self.toggle_auto_followup()
                }
                KeyCode::Char('l') if key.modifiers == KeyModifiers::CONTROL => {
                    self.start_max_auto_turns_edit()
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

        if self.shell_overlay == ShellOverlay::PlanningInit {
            match self.planning_init_overlay_ui_state.step() {
                PlanningInitOverlayStep::ModeSelection => match key.code {
                    KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                        self.planning_init_overlay_ui_state.move_mode_selection(-1)
                    }
                    KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                        self.planning_init_overlay_ui_state.move_mode_selection(1)
                    }
                    KeyCode::Char('a') | KeyCode::Char('A')
                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        self.planning_init_overlay_ui_state
                            .select_mode(PlanningInitModeSelection::Simple)
                    }
                    KeyCode::Char('b') | KeyCode::Char('B')
                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        self.planning_init_overlay_ui_state
                            .select_mode(PlanningInitModeSelection::Detail)
                    }
                    KeyCode::Enter if key.modifiers.is_empty() => {
                        match self.planning_init_overlay_ui_state.selected_mode() {
                            PlanningInitModeSelection::Simple => {
                                self.stage_simple_mode_planning_init_draft()
                            }
                            PlanningInitModeSelection::Detail => {
                                self.planning_init_overlay_ui_state.open_detail_selection()
                            }
                        }
                    }
                    _ => {}
                },
                PlanningInitOverlayStep::DetailSelection => match key.code {
                    KeyCode::Backspace | KeyCode::Left if key.modifiers.is_empty() => self
                        .planning_init_overlay_ui_state
                        .return_to_mode_selection(),
                    KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => self
                        .planning_init_overlay_ui_state
                        .move_detail_selection(-1),
                    KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                        self.planning_init_overlay_ui_state.move_detail_selection(1)
                    }
                    KeyCode::Char('a') | KeyCode::Char('A')
                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        self.planning_init_overlay_ui_state
                            .select_detail(PlanningInitDetailSelection::Manual)
                    }
                    KeyCode::Char('b') | KeyCode::Char('B')
                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        self.planning_init_overlay_ui_state
                            .select_detail(PlanningInitDetailSelection::LlmAssisted)
                    }
                    KeyCode::Enter if key.modifiers.is_empty() => {
                        match self.planning_init_overlay_ui_state.selected_detail() {
                            PlanningInitDetailSelection::Manual => {
                                self.open_planning_manual_editor();
                            }
                            PlanningInitDetailSelection::LlmAssisted => {
                                self.dispatch_conversation_input(
                                    ConversationInputEvent::StatusMessageShown {
                                        status_text:
                                            "planning llm-assisted detail mode is not supported yet"
                                                .to_string(),
                                    },
                                );
                            }
                        }
                    }
                    _ => {}
                },
                PlanningInitOverlayStep::ManualEditor => {
                    if self.handle_planning_manual_editor_close_confirmation_key(key) {
                        return true;
                    }

                    match key.code {
                        KeyCode::Tab if key.modifiers.is_empty() => {
                            self.planning_draft_editor_ui_state.move_file_selection(1)
                        }
                        KeyCode::BackTab => {
                            self.planning_draft_editor_ui_state.move_file_selection(-1)
                        }
                        KeyCode::Left if key.modifiers.is_empty() => {
                            self.planning_draft_editor_ui_state.move_cursor_left()
                        }
                        KeyCode::Right if key.modifiers.is_empty() => {
                            self.planning_draft_editor_ui_state.move_cursor_right()
                        }
                        KeyCode::Up if key.modifiers.is_empty() => {
                            self.planning_draft_editor_ui_state.move_cursor_up()
                        }
                        KeyCode::Down if key.modifiers.is_empty() => {
                            self.planning_draft_editor_ui_state.move_cursor_down()
                        }
                        KeyCode::Enter if key.modifiers.is_empty() => {
                            self.planning_draft_editor_ui_state.insert_newline()
                        }
                        KeyCode::Backspace if key.modifiers.is_empty() => {
                            self.planning_draft_editor_ui_state.backspace()
                        }
                        KeyCode::Char('w') if key.modifiers == KeyModifiers::CONTROL => {
                            self.planning_draft_editor_ui_state.delete_previous_word()
                        }
                        KeyCode::Char('s') if key.modifiers == KeyModifiers::CONTROL => {
                            self.save_planning_manual_editor()
                        }
                        KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                            self.promote_planning_manual_editor()
                        }
                        KeyCode::Char(character)
                            if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                        {
                            self.planning_draft_editor_ui_state
                                .insert_character(character)
                        }
                        _ => {}
                    }
                }
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
            KeyCode::Char('c') if key.modifiers.is_empty() => self.clear_session_browser_state(),
            KeyCode::Char('/') if key.modifiers.is_empty() => {
                self.start_session_search_query_edit()
            }
            KeyCode::Tab if key.modifiers.is_empty() => self.cycle_session_project_filter(1),
            KeyCode::BackTab => self.cycle_session_project_filter(-1),
            KeyCode::Home if key.modifiers.is_empty() => self.jump_to_first_session(),
            KeyCode::End if key.modifiers.is_empty() => self.jump_to_last_session(),
            KeyCode::Char('g') if key.modifiers.is_empty() => self.jump_to_first_session(),
            KeyCode::Char('G') if key.modifiers == KeyModifiers::SHIFT => {
                self.jump_to_last_session()
            }
            KeyCode::PageUp if key.modifiers.is_empty() => self.move_session_page(-1),
            KeyCode::PageDown if key.modifiers.is_empty() => self.move_session_page(1),
            KeyCode::Char('[') if key.modifiers.is_empty() => self.move_session_page(-1),
            KeyCode::Char(']') if key.modifiers.is_empty() => self.move_session_page(1),
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

pub(super) fn planning_prompt_context_load_failed(
    error: String,
) -> PlanningPromptContextLoadResult {
    PlanningPromptContextLoadResult::blocked(format!("failed to load planning workspace: {error}"))
}

fn planning_manual_editor_close_warning_status(risk: PlanningDraftEditorCloseRisk) -> String {
    match (
        risk.has_dirty_buffers(),
        risk.has_invalid_staged_draft(),
    ) {
        (true, true) => "planning draft editor close pending; press Esc again or Enter to discard unsaved edits and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (true, false) => "planning draft editor close pending; press Esc again or Enter to discard unsaved edits, or press n to keep editing".to_string(),
        (false, true) => "planning draft editor close pending; press Esc again or Enter to close and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (false, false) => "planning draft editor close pending".to_string(),
    }
}

fn planning_manual_editor_closed_status(risk: PlanningDraftEditorCloseRisk) -> String {
    match (
        risk.has_dirty_buffers(),
        risk.has_invalid_staged_draft(),
    ) {
        (true, true) => "planning draft editor closed; unsaved in-memory edits were discarded and the staged draft still needs validation".to_string(),
        (true, false) => {
            "planning draft editor closed; unsaved in-memory edits were discarded".to_string()
        }
        (false, true) => "planning draft editor closed; invalid staged draft remains in drafts for review".to_string(),
        (false, false) => "planning draft editor closed".to_string(),
    }
}
