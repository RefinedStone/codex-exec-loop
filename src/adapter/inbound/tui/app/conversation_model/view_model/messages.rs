use crate::adapter::inbound::tui::app::shell_presentation::format_conversation_lines;
use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};

use super::ConversationViewModel;

impl ConversationViewModel {
    pub(crate) fn refresh_conversation_lines(&mut self) {
        self.cached_conversation_lines = format_conversation_lines(&self.messages);
    }

    pub(super) fn push_message(&mut self, message: ConversationMessage) {
        self.messages.push(message);
        self.refresh_conversation_lines();
    }

    pub(super) fn push_messages<I>(&mut self, messages: I)
    where
        I: IntoIterator<Item = ConversationMessage>,
    {
        let mut changed = false;
        for message in messages {
            self.messages.push(message);
            changed = true;
        }

        if changed {
            self.refresh_conversation_lines();
        }
    }

    pub(crate) fn append_status_message(&mut self, text: impl Into<String>) -> bool {
        let text = text.into();
        if text.trim().is_empty() {
            return false;
        }

        if self.messages.last().is_some_and(|message| {
            message.kind == ConversationMessageKind::Status && message.text == text
        }) {
            return false;
        }

        self.push_message(ConversationMessage::new(
            ConversationMessageKind::Status,
            text,
            None,
            None,
        ));
        true
    }

    pub(crate) fn buffer_tool_message(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }

        self.buffered_tool_messages.push(ConversationMessage::new(
            ConversationMessageKind::Tool,
            text,
            None,
            None,
        ));
    }

    pub(crate) fn flush_buffered_tool_messages(&mut self) -> bool {
        if self.buffered_tool_messages.is_empty() {
            return false;
        }

        let buffered_messages = std::mem::take(&mut self.buffered_tool_messages);
        self.push_messages(buffered_messages);
        true
    }

    pub(crate) fn push_live_agent_delta(
        &mut self,
        item_id: String,
        phase: Option<String>,
        delta: String,
    ) {
        if let Some(message) = self.live_agent_message.as_mut()
            && message.item_id.as_deref() == Some(item_id.as_str())
        {
            message.text.push_str(&delta);
            if phase.is_some() {
                message.phase = phase;
            }
            return;
        }

        self.commit_live_agent_message();
        self.live_agent_message = Some(ConversationMessage::new(
            ConversationMessageKind::Agent,
            delta,
            phase,
            Some(item_id),
        ));
    }

    pub(crate) fn complete_live_agent_message(
        &mut self,
        item_id: String,
        phase: Option<String>,
        text: String,
    ) -> bool {
        if let Some(mut message) = self.live_agent_message.take() {
            if message.item_id.as_deref() == Some(item_id.as_str()) {
                message.text = text;
                message.phase = phase;
                self.push_message(message);
                return true;
            }

            self.push_message(message);
        }

        if let Some(message) = self
            .messages
            .iter_mut()
            .rev()
            .find(|message| message.item_id.as_deref() == Some(item_id.as_str()))
        {
            message.text = text;
            message.phase = phase;
            self.refresh_conversation_lines();
            return true;
        }

        self.push_message(ConversationMessage::new(
            ConversationMessageKind::Agent,
            text,
            phase,
            Some(item_id),
        ));
        true
    }

    pub(crate) fn commit_live_agent_message(&mut self) -> bool {
        let Some(message) = self.live_agent_message.take() else {
            return false;
        };

        self.push_message(message);
        true
    }

    pub(crate) fn latest_agent_message_text(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|message| {
                message.kind == ConversationMessageKind::Agent && !message.text.trim().is_empty()
            })
            .map(|message| message.text.as_str())
    }

    pub(crate) fn latest_user_message_text(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|message| {
                message.kind == ConversationMessageKind::User && !message.text.trim().is_empty()
            })
            .map(|message| message.text.as_str())
    }
}
