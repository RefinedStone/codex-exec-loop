use std::time::Instant;

use crossterm::event::{self, KeyCode, KeyModifiers};

use super::super::{
    ConversationState, DEFAULT_AUTO_FOLLOW_MAX_TURNS, FollowupControlEvent, FollowupOverlayUiEvent,
    NativeTuiApp, PlanningInitOverlayStep, ShellOverlay,
};

impl NativeTuiApp {
    pub(crate) fn pause_post_turn_continuation(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::AutoFollowPaused);
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

    pub(crate) fn start_max_auto_turns_edit(&mut self) {
        if !matches!(self.conversation_state, ConversationState::Ready(_)) {
            return;
        }

        if self.shell_overlay != ShellOverlay::PlanningInit
            || self.planning_init_overlay_ui_state.step() != PlanningInitOverlayStep::SimpleReview
        {
            return;
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

    pub(crate) fn handle_max_auto_turns_editor_key(&mut self, key: event::KeyEvent) -> bool {
        if !self.is_max_auto_turns_editing() {
            return false;
        }

        let editor_supported = self.shell_overlay == ShellOverlay::PlanningInit
            && self.planning_init_overlay_ui_state.step() == PlanningInitOverlayStep::SimpleReview;
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
}
