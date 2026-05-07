/*
 * Auto-follow controller code sits at the terminal-input edge of NativeTuiApp.
 * It deliberately avoids owning policy: conversation-level auto-follow changes
 * are sent to `auto_follow_controls`, while in-progress editor text is sent to
 * `auto_follow_overlay_ui`. Keeping those two event streams separate lets the TUI
 * offer a forgiving inline text editor without letting half-typed values change
 * the runtime continuation budget.
 */
use std::time::Instant;

/*
 * This adapter receives already-decoded crossterm keys from the shell router.
 * The mapping below is part of the UI contract for the SimpleReview inline
 * control: Enter commits, Esc/Ctrl-C cancel, Backspace edits local text, and
 * accepted characters stay local until the control reducer validates them.
 */
use crossterm::event::{self, KeyCode, KeyModifiers};

use super::super::{
    AutoFollowControlEvent, AutoFollowOverlayUiEvent, ConversationState,
    DEFAULT_AUTO_FOLLOW_MAX_TURNS, NativeTuiApp, PlanningInitOverlayStep, ShellOverlay,
};

impl NativeTuiApp {
    pub(crate) fn pause_post_turn_continuation(&mut self) {
        /*
         * Pause is an operator intent against the auto-follow policy, not a
         * visual toggle. Sending it through the control reducer keeps footer
         * copy, post-turn continuation guards, and budget accounting on the
         * same ConversationViewModel state.
         */
        self.dispatch_auto_follow_controls(AutoFollowControlEvent::AutoFollowPaused);
    }

    pub(crate) fn current_max_auto_turns_label(&self) -> String {
        /*
         * Ready conversation state is the only canonical owner of
         * max_auto_turns. Startup and failure screens still need stable copy
         * for the editor/status surface, so they fall back to the repository
         * default instead of inventing a separate overlay default.
         */
        match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.auto_follow_state.max_auto_turns_label()
            }
            ConversationState::Loading | ConversationState::Failed(_) => {
                DEFAULT_AUTO_FOLLOW_MAX_TURNS.to_string()
            }
        }
    }

    pub(crate) fn planning_worker_shows_debug_details(&self) -> bool {
        /*
         * Rendering code asks for a capability-shaped bool rather than
         * matching planning_worker_visibility internals. That keeps presentation
         * modules independent from the state machine that decides which
         * planning diagnostics are operator-facing.
         */
        self.planning_worker_visibility.shows_debug_details()
    }

    pub(crate) fn live_activity_pulse(&self, now: Instant) -> Option<u64> {
        /*
         * The footer pulse is derived only when a ready conversation has a
         * live activity timestamp. Loading/failed states have no transcript
         * runtime to measure, so None suppresses the row instead of showing a
         * stale or synthetic duration.
         */
        let conversation_pulse = match &self.conversation_state {
            ConversationState::Ready(conversation) => conversation
                // ConversationViewModel owns the monotonic start instant for the current auto-follow/live turn.
                .live_activity_started_at()
                // Saturating math prevents clock/test anomalies from producing negative-looking elapsed values.
                .map(|started_at| now.saturating_duration_since(started_at).as_secs()),
            ConversationState::Loading | ConversationState::Failed(_) => None,
        };
        if conversation_pulse.is_some() {
            return conversation_pulse;
        }

        if self.parallel_mode_activity_pulse_visible() {
            return Some(0);
        }

        None
    }

    pub(crate) fn is_max_auto_turns_editing(&self) -> bool {
        /*
         * Editing ownership is screen-local state. The conversation may hold
         * a valid max_auto_turns value while the operator is temporarily
         * typing an empty string, `inf`, or another not-yet-valid candidate.
         */
        self.auto_follow_overlay_ui_state
            .max_auto_turns_editor
            .is_editing
    }

    pub(crate) fn start_max_auto_turns_edit(&mut self) {
        /*
         * The editor commits into the active conversation policy, so opening
         * it without a Ready conversation would create an input surface with
         * no durable target. The startup/failure presentations therefore stay
         * read-only for this control.
         */
        if !matches!(self.conversation_state, ConversationState::Ready(_)) {
            return;
        }

        /*
         * The max-turns control lives inside PlanningInit::SimpleReview. This
         * guard prevents a stale keybinding from stealing input while another
         * overlay or another planning-init step owns the keyboard.
         */
        if self.shell_overlay != ShellOverlay::PlanningInit
            || self.planning_init_overlay_ui_state.step() != PlanningInitOverlayStep::SimpleReview
        {
            return;
        }

        /*
         * Starting an edit snapshots the current canonical label into the
         * overlay buffer. From this point until commit/cancel, the buffer is
         * allowed to diverge from the conversation policy.
         */
        self.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::MaxAutoTurnsEditStarted {
            current_value: self.current_max_auto_turns_label(),
        });
    }

    pub(crate) fn save_max_auto_turns_edit(&mut self) {
        /*
         * Enter outside editor ownership must not rewrite policy. The same
         * method is callable from key routing and shell-level flows, so the
         * local guard keeps accidental commits idempotent.
         */
        if !self.is_max_auto_turns_editing() {
            return;
        }

        /*
         * Commit sends the raw buffer to the control reducer. That reducer
         * centralizes normalization and validation, then emits a UI effect
         * only when the policy accepted a canonical value.
         */
        self.dispatch_auto_follow_controls(AutoFollowControlEvent::MaxAutoTurnsUpdated {
            value: self
                .auto_follow_overlay_ui_state
                .max_auto_turns_editor
                .buffer
                .clone(),
        });
    }

    pub(crate) fn cancel_max_auto_turns_edit(&mut self) {
        /*
         * Cancel is purely presentational: close the editor and restore the
         * visible buffer from the current policy label. It intentionally does
         * not dispatch a control event because no policy decision changed.
         */
        self.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::MaxAutoTurnsEditCanceled {
            current_value: self.current_max_auto_turns_label(),
        });
    }

    pub(crate) fn push_max_auto_turns_character(&mut self, character: char) {
        /*
         * Typing appends to overlay state only. Deferring parse errors until
         * save gives this terminal control normal text-editor behavior rather
         * than rejecting intermediate input one key at a time.
         */
        self.dispatch_auto_follow_overlay_ui(
            AutoFollowOverlayUiEvent::MaxAutoTurnsCharacterTyped { character },
        );
    }

    pub(crate) fn pop_max_auto_turns_character(&mut self) {
        /*
         * Backspace edits the same overlay buffer and leaves the canonical
         * auto-follow limit untouched until a later successful save.
         */
        self.dispatch_auto_follow_overlay_ui(
            AutoFollowOverlayUiEvent::MaxAutoTurnsBackspacePressed,
        );
    }

    pub(crate) fn handle_max_auto_turns_editor_key(&mut self, key: event::KeyEvent) -> bool {
        /*
         * The bool is a key-consumption contract for the outer shell router.
         * `false` means this inline editor is not active and global shortcuts
         * may inspect the key; `true` means the editor owned the key even if
         * the key did not mutate the buffer.
         */
        if !self.is_max_auto_turns_editing() {
            return false;
        }

        /*
         * Editing state can outlive a visual transition for one event loop
         * tick. Rechecking the active overlay and step prevents that stale
         * state from capturing keys meant for another surface.
         */
        let editor_supported = self.shell_overlay == ShellOverlay::PlanningInit
            && self.planning_init_overlay_ui_state.step() == PlanningInitOverlayStep::SimpleReview;
        if !editor_supported {
            return false;
        }

        /*
         * Once the editor is the active owner, unsupported keys are still
         * consumed. Letting arrows or punctuation fall through would make one
         * physical keypress affect both the inline editor context and the
         * surrounding planning overlay.
         */
        match key.code {
            // Plain Enter is the only commit gesture; modified Enter combinations remain available for terminal/platform behavior.
            KeyCode::Enter if key.modifiers.is_empty() => self.save_max_auto_turns_edit(),
            // Esc follows the TUI convention of abandoning the active inline edit.
            KeyCode::Esc => self.cancel_max_auto_turns_edit(),
            // Ctrl-C mirrors Esc here so the operator has a terminal-native escape hatch from editor ownership.
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.cancel_max_auto_turns_edit()
            }
            // Backspace is routed through the UI reducer so empty-buffer behavior stays centralized.
            KeyCode::Backspace => self.pop_max_auto_turns_character(),
            /*
             * The canonical parser accepts numeric labels and named forms
             * such as `infinite`; key routing therefore permits ASCII
             * alphanumerics and leaves semantic validation to commit time.
             */
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
