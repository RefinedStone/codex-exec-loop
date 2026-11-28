use std::time::{Duration, Instant};

use super::fixture::{client_fixture, conversation};
use crate::adapter::outbound::gateway_kernel_client::KernelValueUpdate;
use crate::core::app::{AppCommand, ConversationSnapshot, CoreInput};
use crate::core::native_client_port::{NativeClientEvent, NativeClientPort};

fn load(thread_id: &str) -> NativeClientEvent {
    NativeClientEvent::core(CoreInput::Command(AppCommand::LoadConversation {
        thread_id: thread_id.into(),
        fallback_workspace_directory: "/repo".into(),
    }))
}

#[test]
fn changing_selection_discards_the_earlier_gateway_history_result() {
    let (mut client, _updates) = client_fixture(
        "/repo",
        vec![
            ("first".into(), conversation("first")),
            ("second".into(), conversation("second")),
        ],
    );
    let _ = client.dispatch_client_event(load("first"));
    let _ = client.dispatch_client_event(load("second"));
    assert!(matches!(
        client.snapshot().conversation,
        ConversationSnapshot::Loading
    ));

    assert!(
        client
            .run_host_cycle(Instant::now(), Duration::from_secs(2), 1)
            .is_empty()
    );
    let outcomes = client.run_host_cycle(Instant::now(), Duration::from_secs(2), 1);
    assert_eq!(outcomes.len(), 1);
    let ConversationSnapshot::Ready(ready) = client.snapshot().conversation else {
        panic!("the selected history must become ready");
    };
    assert_eq!(ready.thread_id, "second");
    assert_eq!(ready.conversation, Box::new(conversation("second")));
}

#[test]
fn leaving_the_view_discards_an_inflight_read() {
    let (mut client, _updates) =
        client_fixture("/repo", vec![("first".into(), conversation("first"))]);
    let _ = client.dispatch_client_event(load("first"));
    let _ = client.dispatch_client_event(NativeClientEvent::core(CoreInput::Command(
        AppCommand::InvalidateConversationLoad,
    )));
    assert!(
        client
            .run_host_cycle(Instant::now(), Duration::from_secs(2), 1)
            .is_empty()
    );
    assert_eq!(client.snapshot().conversation, ConversationSnapshot::Idle);
}

#[test]
fn a_provider_response_for_another_thread_is_not_displayed() {
    let (mut client, _updates) =
        client_fixture("/repo", vec![("requested".into(), conversation("foreign"))]);
    let _ = client.dispatch_client_event(load("requested"));
    client.run_host_cycle(Instant::now(), Duration::from_secs(2), 1);
    assert!(matches!(
        client.snapshot().conversation,
        ConversationSnapshot::Failed { message } if message.contains("different thread")
    ));
}

#[test]
fn gateway_disconnect_invalidates_a_pending_history_read() {
    let (mut client, updates) =
        client_fixture("/repo", vec![("first".into(), conversation("first"))]);
    updates
        .send(KernelValueUpdate::Disconnected("connection lost".into()).into())
        .unwrap();
    let _ = client.dispatch_client_event(load("first"));
    client.run_host_cycle(Instant::now(), Duration::from_secs(2), 1);
    assert!(
        client
            .run_host_cycle(Instant::now(), Duration::from_secs(2), 1)
            .is_empty()
    );
    assert!(matches!(
        client.snapshot().conversation,
        ConversationSnapshot::Failed { message } if message.contains("connection lost")
    ));
}
