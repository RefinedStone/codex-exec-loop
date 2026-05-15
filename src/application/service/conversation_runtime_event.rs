use std::sync::mpsc::Sender;

use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

pub use crate::domain::conversation_stream::ConversationStreamEvent;

pub(crate) fn emit_attachment_observed(
    event_sender: &Sender<ConversationStreamEvent>,
    profile: TerminalBridgeAttachmentProfile,
) {
    let _ = event_sender.send(ConversationStreamEvent::attachment_observed(profile));
}

pub(crate) fn emit_codex_app_server_launch_attachment(
    event_sender: &Sender<ConversationStreamEvent>,
) {
    emit_attachment_observed(
        event_sender,
        TerminalBridgeAttachmentProfile::codex_app_server_launch(),
    );
}

pub(crate) fn emit_codex_app_server_reattach_attachment(
    event_sender: &Sender<ConversationStreamEvent>,
) {
    emit_attachment_observed(
        event_sender,
        TerminalBridgeAttachmentProfile::codex_app_server_reattach(),
    );
}

#[cfg(test)]
mod tests {
    use super::ConversationStreamEvent;
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    #[test]
    fn codex_attachment_helpers_build_expected_profiles() {
        assert_eq!(
            ConversationStreamEvent::codex_app_server_launch_attachment(),
            ConversationStreamEvent::AttachmentObserved {
                profile: TerminalBridgeAttachmentProfile::codex_app_server_launch(),
            }
        );
        assert_eq!(
            ConversationStreamEvent::codex_app_server_reattach_attachment(),
            ConversationStreamEvent::AttachmentObserved {
                profile: TerminalBridgeAttachmentProfile::codex_app_server_reattach(),
            }
        );
    }
}
