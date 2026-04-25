#[cfg(test)]
use super::Line;
#[cfg(test)]
use super::Rect;
use super::capability_projection::recent_session_status_label;
use super::{
    ConversationState, ConversationViewModel, NativeTuiApp, ShellActionAvailability, StartupState,
};

#[cfg(test)]
pub(in super::super) struct ConversationShellView {
    pub(in super::super) shell_title: Line<'static>,
    pub(in super::super) header_lines: Vec<Line<'static>>,
    pub(in super::super) conversation_lines: Vec<Line<'static>>,
    pub(in super::super) status_title: Line<'static>,
    pub(in super::super) footer_lines: Vec<Line<'static>>,
    pub(in super::super) input_title: Line<'static>,
    pub(in super::super) input_lines: Vec<Line<'static>>,
}

#[cfg(test)]
#[allow(dead_code)]
pub(in super::super) struct ConversationShellFrameView {
    pub(in super::super) shell_title: Line<'static>,
    pub(in super::super) header_lines: Vec<Line<'static>>,
    pub(in super::super) header_area: Rect,
    pub(in super::super) transcript_view: TranscriptPanelView,
    pub(in super::super) transcript_area: Rect,
    pub(in super::super) status_title: Line<'static>,
    pub(in super::super) footer_lines: Vec<Line<'static>>,
    pub(in super::super) footer_area: Rect,
    pub(in super::super) input_title: Line<'static>,
    pub(in super::super) input_lines: Vec<Line<'static>>,
    pub(in super::super) input_area: Rect,
}

#[cfg(test)]
pub(in super::super) struct TranscriptPanelView {
    pub(in super::super) title: Line<'static>,
    pub(in super::super) lines: Vec<Line<'static>>,
    pub(in super::super) scroll_offset: u16,
}

#[derive(Clone, Copy)]
pub(super) enum ShellConversationState<'a> {
    Loading,
    Failed(&'a str),
    Ready(&'a ConversationViewModel),
}

pub(super) struct ShellCorePresentationContext<'a> {
    pub(super) show_startup_ascii_art: bool,
    pub(super) startup_state: &'a StartupState,
    pub(super) shell_action_availability: ShellActionAvailability,
    pub(super) recent_session_status_label: String,
    pub(super) github_review_polling_status_label: String,
    #[cfg(test)]
    pub(super) planner_shows_debug_details: bool,
    pub(super) conversation_state: ShellConversationState<'a>,
}

impl<'a> ShellCorePresentationContext<'a> {
    pub(super) fn from_app(app: &'a NativeTuiApp) -> Self {
        Self {
            show_startup_ascii_art: app.show_startup_ascii_art,
            startup_state: &app.startup_state,
            shell_action_availability: app.shell_action_availability(),
            recent_session_status_label: recent_session_status_label(app),
            github_review_polling_status_label: app.github_review_polling_status_label(),
            #[cfg(test)]
            planner_shows_debug_details: app.planner_shows_debug_details(),
            conversation_state: match &app.conversation_state {
                ConversationState::Loading => ShellConversationState::Loading,
                ConversationState::Failed(message) => ShellConversationState::Failed(message),
                ConversationState::Ready(conversation) => {
                    ShellConversationState::Ready(conversation)
                }
            },
        }
    }

    pub(super) fn ready_conversation(&self) -> Option<&'a ConversationViewModel> {
        match self.conversation_state {
            ShellConversationState::Ready(conversation) => Some(conversation),
            _ => None,
        }
    }

    pub(super) fn startup_screen_is_active(&self) -> bool {
        let Some(conversation) = self.ready_conversation() else {
            return false;
        };

        !conversation.has_active_thread()
            && conversation.messages.is_empty()
            && conversation.active_turn_id.is_none()
            && conversation.live_agent_message.is_none()
    }

    pub(super) fn startup_banner_is_active(&self) -> bool {
        self.show_startup_ascii_art && self.startup_screen_is_active()
    }
}
