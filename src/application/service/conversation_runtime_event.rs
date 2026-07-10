use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

pub use crate::domain::conversation_stream::ConversationStreamEvent;

// Keep only a short provider burst between the app-server producer and the
// application reducer. Ordinary stream events use FIFO backpressure. Interactive
// approvals use `try_send` at the adapter boundary and are declined when this
// queue is full, so a control request can never wait indefinitely behind output.
pub const CONVERSATION_STREAM_CHANNEL_CAPACITY: usize = 8;
pub type ConversationStreamSender = SyncSender<ConversationStreamEvent>;
pub type ConversationStreamReceiver = Receiver<ConversationStreamEvent>;

pub fn conversation_stream_channel() -> (ConversationStreamSender, ConversationStreamReceiver) {
    sync_channel(CONVERSATION_STREAM_CHANNEL_CAPACITY)
}

pub(crate) fn emit_attachment_observed(
    event_sender: &ConversationStreamSender,
    profile: TerminalBridgeAttachmentProfile,
) {
    let _ = event_sender.send(ConversationStreamEvent::attachment_observed(profile));
}

pub(crate) fn emit_codex_app_server_launch_attachment(event_sender: &ConversationStreamSender) {
    emit_attachment_observed(
        event_sender,
        TerminalBridgeAttachmentProfile::codex_app_server_launch(),
    );
}

pub(crate) fn emit_codex_app_server_reattach_attachment(event_sender: &ConversationStreamSender) {
    emit_attachment_observed(
        event_sender,
        TerminalBridgeAttachmentProfile::codex_app_server_reattach(),
    );
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::TrySendError;

    use super::{
        CONVERSATION_STREAM_CHANNEL_CAPACITY, ConversationStreamEvent, conversation_stream_channel,
    };
    use crate::domain::conversation::{
        ConversationApprovalRequest, ConversationApprovalRequestKind,
    };
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

    #[test]
    fn conversation_stream_channel_applies_fifo_backpressure_and_disconnects() {
        let (sender, receiver) = conversation_stream_channel();
        for sequence in 0..CONVERSATION_STREAM_CHANNEL_CAPACITY - 1 {
            sender
                .try_send(ConversationStreamEvent::StatusUpdated {
                    text: sequence.to_string(),
                })
                .expect("events within the fixed capacity should be admitted");
        }
        let approval = ConversationApprovalRequest {
            approval_id: "approval-1".to_string(),
            server_request_id: "request-1".to_string(),
            method: "item/commandExecution/requestApproval".to_string(),
            kind: ConversationApprovalRequestKind::CommandExecution,
            summary: "approve command".to_string(),
            details: vec!["cargo test".to_string()],
        };
        sender
            .try_send(ConversationStreamEvent::ApprovalRequested {
                request: approval.clone(),
            })
            .expect("approval control event should retain FIFO admission");

        assert!(matches!(
            sender.try_send(ConversationStreamEvent::StatusUpdated {
                text: "overflow".to_string(),
            }),
            Err(TrySendError::Full(_))
        ));
        for sequence in 0..CONVERSATION_STREAM_CHANNEL_CAPACITY - 1 {
            assert_eq!(
                receiver
                    .recv()
                    .expect("queued event should remain available"),
                ConversationStreamEvent::StatusUpdated {
                    text: sequence.to_string(),
                }
            );
        }
        assert_eq!(
            receiver
                .recv()
                .expect("approval control event should remain queued"),
            ConversationStreamEvent::ApprovalRequested { request: approval }
        );

        drop(receiver);
        assert!(
            sender
                .send(ConversationStreamEvent::StatusUpdated {
                    text: "disconnected".to_string(),
                })
                .is_err()
        );
    }
}
