use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ConversationViewMode {
    #[default]
    Simple,
    Medium,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ViewSelectionModeOption {
    pub(super) mode: ConversationViewMode,
    pub(super) detail: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ViewSelectionOverlayUiState {
    selected_mode_index: usize,
}

pub(super) const VIEW_SELECTION_MODE_OPTIONS: &[ViewSelectionModeOption] = &[
    ViewSelectionModeOption {
        mode: ConversationViewMode::Simple,
        detail: "Show user prompts, Codex, and Codex Commentary only.",
    },
    ViewSelectionModeOption {
        mode: ConversationViewMode::Medium,
        detail: "Also show tool and shell status transcript rows.",
    },
    ViewSelectionModeOption {
        mode: ConversationViewMode::Detail,
        detail: "Show tool/status rows plus diagnostic debug detail.",
    },
];

impl ConversationViewMode {
    pub(super) const SUPPORTED_LABELS: &'static str = "simple, medium, detail";

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "simple" => Some(Self::Simple),
            // `midium` is accepted as an input alias for the original operator request;
            // UI labels and help text keep the canonical `medium` spelling.
            "medium" | "midium" => Some(Self::Medium),
            "detail" | "detailed" => Some(Self::Detail),
            _ => None,
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Medium => "medium",
            Self::Detail => "detail",
        }
    }

    pub(super) const fn shows_debug_details(self) -> bool {
        matches!(self, Self::Detail)
    }

    pub(super) fn includes_message(self, message: &ConversationMessage) -> bool {
        match message.kind {
            ConversationMessageKind::User | ConversationMessageKind::Agent => true,
            ConversationMessageKind::Tool | ConversationMessageKind::Status => {
                matches!(self, Self::Medium | Self::Detail)
            }
        }
    }
}

impl ViewSelectionOverlayUiState {
    pub(super) fn reset_from_mode(&mut self, mode: ConversationViewMode) {
        self.selected_mode_index = mode_option_index(mode).unwrap_or(0);
    }

    pub(super) fn selected_mode_index(&self) -> usize {
        self.selected_mode_index
    }

    pub(super) fn selected_mode(&self) -> ConversationViewMode {
        VIEW_SELECTION_MODE_OPTIONS[self.selected_mode_index].mode
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        let len = VIEW_SELECTION_MODE_OPTIONS.len();
        if len == 0 {
            return;
        }
        let next = (self.selected_mode_index as isize + delta).rem_euclid(len as isize) as usize;
        self.selected_mode_index = next;
    }

    pub(super) fn select_index(&mut self, index: usize) -> bool {
        if index >= VIEW_SELECTION_MODE_OPTIONS.len() {
            return false;
        }
        self.selected_mode_index = index;
        true
    }
}

fn mode_option_index(mode: ConversationViewMode) -> Option<usize> {
    VIEW_SELECTION_MODE_OPTIONS
        .iter()
        .position(|option| option.mode == mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_accepts_supported_labels_and_midium_alias() {
        assert_eq!(
            ConversationViewMode::parse("simple"),
            Some(ConversationViewMode::Simple)
        );
        assert_eq!(
            ConversationViewMode::parse("midium"),
            Some(ConversationViewMode::Medium)
        );
        assert_eq!(
            ConversationViewMode::parse("detail"),
            Some(ConversationViewMode::Detail)
        );
        assert_eq!(ConversationViewMode::parse("tool"), None);
    }

    #[test]
    fn simple_keeps_user_and_agent_messages_only() {
        let user = ConversationMessage::new(ConversationMessageKind::User, "hi", None, None);
        let agent = ConversationMessage::new(ConversationMessageKind::Agent, "ok", None, None);
        let tool = ConversationMessage::new(ConversationMessageKind::Tool, "cmd", None, None);
        let status = ConversationMessage::new(ConversationMessageKind::Status, "idle", None, None);

        assert!(ConversationViewMode::Simple.includes_message(&user));
        assert!(ConversationViewMode::Simple.includes_message(&agent));
        assert!(!ConversationViewMode::Simple.includes_message(&tool));
        assert!(!ConversationViewMode::Simple.includes_message(&status));
    }
}
