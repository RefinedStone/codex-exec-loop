use std::time::Instant;

use crossterm::event::{self, KeyCode, KeyModifiers};

use super::super::{
    ConversationInputEvent, ConversationState, DEFAULT_AUTO_FOLLOW_MAX_TURNS,
    DEFAULT_AUTO_FOLLOW_STOP_KEYWORD, FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP, FollowupControlEvent,
    FollowupOverlayUiEvent, NativeTuiApp, PlanningInitOverlayStep, ShellChromeEvent, ShellOverlay,
};

impl NativeTuiApp {
    pub(crate) fn reload_followup_templates(&mut self) {
        let workspace_directory = match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.planning_workspace_directory().to_string()
            }
            ConversationState::Loading | ConversationState::Failed(_) => return,
        };

        self.dispatch_followup_controls(FollowupControlEvent::TemplateCatalogReloaded {
            reload_result: self
                .followup_template_service
                .reload_catalog(&workspace_directory),
        });
    }

    pub(crate) fn show_followup_template_overlay(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::OverlayShown {
            stop_keyword: self.current_stop_keyword_value(),
            max_auto_turns: self.current_max_auto_turns_label(),
        });
        self.dispatch_shell_chrome(ShellChromeEvent::FollowupTemplatesOverlayShown);
    }

    pub(crate) fn toggle_followup_template_overlay(&mut self) {
        if self.shell_overlay != ShellOverlay::FollowupTemplates {
            self.show_followup_template_overlay();
            return;
        }
        self.dispatch_shell_chrome(ShellChromeEvent::FollowupTemplatesOverlayToggled);
    }

    pub(crate) fn toggle_auto_followup(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::AutoFollowToggled);
    }

    pub(crate) fn stop_post_turn_automation(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::AutoFollowStopped);
    }

    pub(crate) fn current_max_auto_turns_label(&self) -> String {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.auto_follow_state.max_auto_turns_label()
            }
            ConversationState::Loading | ConversationState::Failed(_) => {
                DEFAULT_AUTO_FOLLOW_MAX_TURNS.to_string()
            }
        }
    }

    pub(crate) fn current_stop_keyword_value(&self) -> String {
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

    pub(crate) fn planner_visibility_label(&self) -> &'static str {
        self.planner_visibility.label()
    }

    pub(crate) fn planner_shows_debug_details(&self) -> bool {
        self.planner_visibility.shows_debug_details()
    }

    pub(crate) fn live_activity_pulse(&self, now: Instant) -> Option<u64> {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => conversation
                .live_activity_started_at()
                .map(|started_at| now.saturating_duration_since(started_at).as_secs()),
            ConversationState::Loading | ConversationState::Failed(_) => None,
        }
    }

    pub(crate) fn is_max_auto_turns_editing(&self) -> bool {
        self.followup_overlay_ui_state
            .max_auto_turns_editor
            .is_editing
    }

    pub(crate) fn is_stop_keyword_editing(&self) -> bool {
        self.followup_overlay_ui_state
            .stop_keyword_editor
            .is_editing
    }

    pub(crate) fn start_max_auto_turns_edit(&mut self) {
        if !matches!(self.conversation_state, ConversationState::Ready(_)) {
            return;
        }

        if self.shell_overlay != ShellOverlay::FollowupTemplates
            && self.shell_overlay != ShellOverlay::PlanningInit
        {
            self.show_followup_template_overlay();
        }

        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsEditStarted {
            current_value: self.current_max_auto_turns_label(),
        });
    }

    pub(crate) fn save_max_auto_turns_edit(&mut self) {
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

    pub(crate) fn cancel_max_auto_turns_edit(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsEditCanceled {
            current_value: self.current_max_auto_turns_label(),
        });
    }

    pub(crate) fn push_max_auto_turns_character(&mut self, character: char) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsCharacterTyped {
            character,
        });
    }

    pub(crate) fn pop_max_auto_turns_character(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsBackspacePressed);
    }

    pub(crate) fn start_stop_keyword_edit(&mut self) {
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

    pub(crate) fn save_stop_keyword_edit(&mut self) {
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

    pub(crate) fn cancel_stop_keyword_edit(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordEditCanceled {
            current_value: self.current_stop_keyword_value(),
        });
    }

    pub(crate) fn push_stop_keyword_character(&mut self, character: char) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordCharacterTyped {
            character,
        });
    }

    pub(crate) fn pop_stop_keyword_character(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordBackspacePressed);
    }

    pub(crate) fn toggle_stop_keyword(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::StopKeywordToggled);
    }

    pub(crate) fn toggle_no_file_change_stop(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::NoFileChangeStopToggled);
    }

    pub(crate) fn toggle_planner_visibility(&mut self) {
        self.planner_visibility = self.planner_visibility.toggle();
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: format!("planner detail {}", self.planner_visibility.label()),
        });
    }

    pub(crate) fn cycle_auto_followup_template(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::TemplateCycledForward);
    }

    pub(crate) fn cycle_auto_followup_template_backward(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::TemplateCycledBackward);
    }

    #[cfg(test)]
    pub(crate) fn followup_template_selection(&self) -> Option<usize> {
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

    pub(crate) fn scroll_followup_template_preview(&mut self, delta: i32) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::PreviewScrolled { delta });
    }

    pub(crate) fn handle_stop_keyword_editor_key(&mut self, key: event::KeyEvent) -> bool {
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

    pub(crate) fn handle_max_auto_turns_editor_key(&mut self, key: event::KeyEvent) -> bool {
        if !self.is_max_auto_turns_editing() {
            return false;
        }

        let editor_supported = self.shell_overlay == ShellOverlay::FollowupTemplates
            || (self.shell_overlay == ShellOverlay::PlanningInit
                && self.planning_init_overlay_ui_state.step()
                    == PlanningInitOverlayStep::SimpleReview);
        if !editor_supported {
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
                    && character.is_ascii_alphanumeric() =>
            {
                self.push_max_auto_turns_character(character);
            }
            _ => {}
        }

        true
    }

    pub(crate) fn handle_followup_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay != ShellOverlay::FollowupTemplates {
            return false;
        }

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
            KeyCode::Char('b') if key.modifiers == KeyModifiers::CONTROL => {
                self.toggle_planner_visibility()
            }
            KeyCode::PageUp if key.modifiers.is_empty() => self
                .scroll_followup_template_preview(-(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32)),
            KeyCode::PageDown if key.modifiers.is_empty() => {
                self.scroll_followup_template_preview(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32)
            }
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => self
                .scroll_followup_template_preview(-(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32)),
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                self.scroll_followup_template_preview(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32)
            }
            KeyCode::Enter if key.modifiers.is_empty() => self.close_shell_overlay(),
            _ => {}
        }

        true
    }
}
