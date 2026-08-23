use std::collections::VecDeque;

use super::{ConversationInputEvent, ConversationState, NativeTuiApp};
use crate::domain::conversation::ConversationMessageKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TerminalUiEffect {
    CopyToClipboard(String),
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct TerminalInteractionUiState {
    pending_effects: VecDeque<TerminalUiEffect>,
}

impl TerminalInteractionUiState {
    fn request_clipboard_copy(&mut self, text: String) {
        self.pending_effects
            .push_back(TerminalUiEffect::CopyToClipboard(text));
    }

    pub(super) fn take_effects(&mut self) -> Vec<TerminalUiEffect> {
        self.pending_effects.drain(..).collect()
    }
}

impl NativeTuiApp {
    pub(super) fn take_terminal_ui_effects(&mut self) -> Vec<TerminalUiEffect> {
        self.shell
            .transcript_viewport_ui_state
            .terminal_interaction_mut()
            .take_effects()
    }

    pub(super) fn request_selection_copy(&mut self, text: String) {
        self.queue_clipboard_copy(text, ClipboardCopySource::Selection);
    }

    pub(super) fn copy_active_transcript_selection(&mut self) -> bool {
        let Some(text) = self
            .shell
            .transcript_viewport_ui_state
            .active_selection_text()
        else {
            return false;
        };
        self.queue_clipboard_copy(text, ClipboardCopySource::Selection);
        true
    }

    pub(super) fn handle_copy_shell_command(&mut self, argument: Option<&str>) {
        let argument = argument.map(str::trim).filter(|value| !value.is_empty());
        let selection = self
            .shell
            .transcript_viewport_ui_state
            .copied_selection_text()
            .map(str::to_string);
        let last_answer = || self.latest_agent_answer_text();
        let requested = match argument.map(str::to_ascii_lowercase).as_deref() {
            None => selection
                .map(|text| (text, ClipboardCopySource::Selection))
                .or_else(|| last_answer().map(|text| (text, ClipboardCopySource::LastAnswer))),
            Some("selection" | "selected") => {
                selection.map(|text| (text, ClipboardCopySource::Selection))
            }
            Some("last" | "answer" | "response") => {
                last_answer().map(|text| (text, ClipboardCopySource::LastAnswer))
            }
            Some(_) => {
                self.show_terminal_interaction_status(
                    self.shell.tui_language.terminal_copy_usage(),
                );
                return;
            }
        };
        let Some((text, source)) = requested else {
            let selection_requested = argument.is_some_and(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "selection" | "selected"
                )
            });
            self.show_terminal_interaction_status(
                self.shell
                    .tui_language
                    .terminal_copy_unavailable(selection_requested),
            );
            return;
        };
        self.queue_clipboard_copy(text, source);
    }

    fn latest_agent_answer_text(&self) -> Option<String> {
        let ConversationState::Ready(conversation) =
            &self.conversation.lifecycle.conversation_state
        else {
            return None;
        };
        conversation
            .messages
            .iter()
            .rev()
            .find(|message| message.kind == ConversationMessageKind::Agent)
            .map(|message| message.text.trim_end().to_string())
            .filter(|text| !text.is_empty())
    }

    fn queue_clipboard_copy(&mut self, text: String, source: ClipboardCopySource) {
        if text.is_empty() {
            return;
        }
        let character_count = text.chars().count();
        self.shell
            .transcript_viewport_ui_state
            .terminal_interaction_mut()
            .request_clipboard_copy(text);
        self.show_terminal_interaction_status(
            self.shell
                .tui_language
                .terminal_copy_requested(source == ClipboardCopySource::Selection, character_count),
        );
    }

    fn show_terminal_interaction_status(&mut self, status_text: String) {
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardCopySource {
    Selection,
    LastAnswer,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;

    #[test]
    fn document_switch_preserves_pending_terminal_copy() {
        let mut app = test_native_tui_app();
        app.request_selection_copy("selected transcript".to_string());

        app.shell
            .transcript_viewport_ui_state
            .bind_document(Some("another-thread".to_string()));

        assert_eq!(
            app.take_terminal_ui_effects(),
            vec![TerminalUiEffect::CopyToClipboard(
                "selected transcript".to_string()
            )]
        );
    }

    #[test]
    fn copy_last_queues_raw_agent_text_without_terminal_formatting() {
        let mut app = test_native_tui_app();
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test conversation should be ready");
        };
        assert!(conversation.finalize_agent_message(
            "agent-copy".to_string(),
            Some("final".to_string()),
            "plain **Markdown** answer".to_string(),
        ));

        app.handle_copy_shell_command(Some("last"));

        assert_eq!(
            app.take_terminal_ui_effects(),
            vec![TerminalUiEffect::CopyToClipboard(
                "plain **Markdown** answer".to_string()
            )]
        );
    }

    #[test]
    fn implicit_copy_falls_back_to_last_answer_but_explicit_selection_does_not() {
        let mut app = test_native_tui_app();
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test conversation should be ready");
        };
        assert!(conversation.finalize_agent_message(
            "agent-copy-fallback".to_string(),
            Some("final".to_string()),
            "fallback answer".to_string(),
        ));

        app.handle_copy_shell_command(Some("selection"));
        assert!(
            app.take_terminal_ui_effects().is_empty(),
            "an explicit selection request must not silently copy a different source"
        );

        app.handle_copy_shell_command(None);
        assert_eq!(
            app.take_terminal_ui_effects(),
            vec![TerminalUiEffect::CopyToClipboard(
                "fallback answer".to_string()
            )]
        );
    }
}
