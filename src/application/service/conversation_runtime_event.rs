use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

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

#[cfg(test)]
pub(crate) fn confirmed_test_terminal_receipt()
-> crate::domain::turn_terminal::ConversationTurnTerminalReceipt {
    use crate::domain::turn_terminal::{
        ConversationTurnApplicationDelivery, ConversationTurnTerminalReceipt,
    };

    ConversationTurnTerminalReceipt::completed("test-thread", "test-turn", Vec::new())
        .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed)
}

#[cfg(test)]
pub(crate) fn emit_confirmed_test_terminal_receipt(
    event_sender: &ConversationStreamSender,
    thread_id: &str,
    cwd: &str,
) -> anyhow::Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
    use crate::domain::turn_terminal::{
        ConversationTurnApplicationDelivery, ConversationTurnTerminalReceipt,
    };

    let turn_id = "test-turn";
    event_sender
        .send(ConversationStreamEvent::ThreadPrepared {
            thread_id: thread_id.to_string(),
            title: "Test thread".to_string(),
            cwd: cwd.to_string(),
        })
        .map_err(|_| anyhow::anyhow!("test stream event receiver disconnected"))?;
    event_sender
        .send(ConversationStreamEvent::TurnStarted {
            turn_id: turn_id.to_string(),
        })
        .map_err(|_| anyhow::anyhow!("test stream event receiver disconnected"))?;
    let receipt = ConversationTurnTerminalReceipt::completed(thread_id, turn_id, Vec::new())
        .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);
    event_sender
        .send(ConversationStreamEvent::TurnTerminal {
            receipt: receipt.clone(),
        })
        .map_err(|_| anyhow::anyhow!("test stream event receiver disconnected"))?;
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::TrySendError;

    use super::{
        CONVERSATION_STREAM_CHANNEL_CAPACITY, ConversationStreamEvent, conversation_stream_channel,
        emit_confirmed_test_terminal_receipt,
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

    #[test]
    fn confirmed_test_stream_emits_correlated_terminal_event_and_return_receipt() {
        let (sender, receiver) = conversation_stream_channel();

        let returned =
            emit_confirmed_test_terminal_receipt(&sender, "thread-test", "/tmp/test-workspace")
                .expect("test terminal stream should emit");

        assert!(matches!(
            receiver.recv().expect("thread event should arrive"),
            ConversationStreamEvent::ThreadPrepared {
                ref thread_id,
                ref cwd,
                ..
            } if thread_id == "thread-test" && cwd == "/tmp/test-workspace"
        ));
        assert!(matches!(
            receiver.recv().expect("turn event should arrive"),
            ConversationStreamEvent::TurnStarted { ref turn_id } if turn_id == "test-turn"
        ));
        assert_eq!(
            receiver.recv().expect("terminal event should arrive"),
            ConversationStreamEvent::TurnTerminal {
                receipt: returned.clone(),
            }
        );
        assert!(returned.is_completed_and_confirmed());
    }
}
