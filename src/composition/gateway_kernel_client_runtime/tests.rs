use crate::core::app::{AppCommand, CoreInput};
use crate::core::native_client_port::{NativeClientEvent, NativeClientPort};
use crate::domain::recent_sessions::SessionCatalog;
use serde_json::json;
use std::time::{Duration, Instant};

use super::*;

#[test]
fn a_lost_gateway_mutation_reply_is_not_replayed_on_reconnect() {
    use crate::application::port::outbound::kernel_gateway_port::{
        KernelGatewayPort, KernelGatewayReply,
    };
    use std::sync::mpsc;

    struct GatewayFixture(mpsc::Sender<Value>);
    impl KernelGatewayPort for GatewayFixture {
        fn request(
            &self,
            method: &str,
            params: Value,
            wait: bool,
        ) -> anyhow::Result<KernelGatewayReply> {
            assert_eq!(method, "akra/kernel/command");
            assert!(!wait);
            self.0.send(params).unwrap();
            anyhow::bail!("fixture accepted the write and lost the reply")
        }
    }
    let (mut client, updates) = super::fixture::client_fixture("/repo", Vec::new());
    let (invoked, requests) = mpsc::channel();
    client.commands = Some(
        super::commands::GatewayCommands::new(
            Arc::new(GatewayFixture(invoked)),
            None,
            updates.clone(),
        )
        .unwrap(),
    );
    let write = || {
        NativeClientEvent::core(CoreInput::Command(AppCommand::SetAutoFollowMaxTurns {
            value: 5,
        }))
    };
    let offline = events(vec![client.dispatch_client_event(write())]);
    assert!(
        matches!(&offline[..], [AppEvent::GatewayCommandStatus(message)] if message.contains("nothing was sent"))
    );
    updates
        .send(KernelClientUpdate::Synchronized.into())
        .unwrap();
    let _sync = client.run_host_cycle(Instant::now(), Duration::ZERO, 1);
    let _admission = client.dispatch_client_event(write());
    let submitted = requests.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        submitted["command"],
        json!({"type":"set_auto_follow_max_turns","value":5})
    );
    assert!(
        submitted["idempotency_key"]
            .as_str()
            .is_some_and(|key| key.len() == 32)
    );
    let lost = events(client.run_host_cycle(Instant::now(), Duration::from_secs(2), 1));
    assert!(
        matches!(&lost[..], [AppEvent::GatewayCommandStatus(message)] if message.contains("outcome unknown") && message.contains("not replayed"))
    );

    updates
        .send(KernelClientUpdate::Disconnected("fixture dropped connection".to_string()).into())
        .unwrap();
    let _disconnect = client.run_host_cycle(Instant::now(), Duration::ZERO, 1);
    let during_outage = events(vec![client.dispatch_client_event(write())]);
    assert!(
        matches!(&during_outage[..], [AppEvent::GatewayCommandStatus(message)] if message.contains("nothing was sent"))
    );
    updates
        .send(KernelClientUpdate::Synchronized.into())
        .unwrap();
    let _reconnect = client.run_host_cycle(Instant::now(), Duration::ZERO, 1);
    let duplicate = events(vec![client.dispatch_client_event(write())]);
    assert!(
        matches!(&duplicate[..], [AppEvent::GatewayCommandStatus(message)] if message.contains("not sent again"))
    );
    drop(client);
    assert_eq!(
        requests.recv_timeout(Duration::from_secs(2)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
}

/// Build a client with fake Kernel values and no provider history.
pub(crate) fn kernel_client_with_values(
    workspace_directory: &str,
    values: Vec<Value>,
) -> GatewayKernelClientRuntime {
    kernel_client_with_history(workspace_directory, values, Vec::new())
}

pub(crate) fn kernel_client_with_history(
    workspace_directory: &str,
    values: Vec<Value>,
    history: Vec<(String, crate::domain::conversation::ConversationSnapshot)>,
) -> GatewayKernelClientRuntime {
    let (client, sender) = super::fixture::client_fixture(workspace_directory, history);
    for value in values {
        sender
            .send(KernelClientUpdate::Value(value).into())
            .unwrap();
    }
    sender
        .send(KernelClientUpdate::Synchronized.into())
        .unwrap();
    client
}

fn events(outcomes: Vec<NativeClientDispatchOutcome>) -> Vec<AppEvent> {
    outcomes
        .into_iter()
        .flat_map(|outcome| match outcome {
            NativeClientDispatchOutcome::Core(core) => core.events,
            _ => panic!("the kernel client only answers with core outcomes"),
        })
        .collect()
}

#[test]
fn kernel_values_become_the_session_list_thread_activity_and_auto_follow_settings() {
    let mut client = kernel_client_with_values(
        "/repo",
        vec![
            json!({"type":"sessions","status":"ready","sessions":[
                {"thread_id":"t-1","title":"Fix CI","workspace":"/repo","updated_at_epoch":7}
            ]}),
            json!({"type":"auto_follow","max_turns":3,"paused":true}),
            json!({"type":"thread_activity","threads":[
                {"thread_id":"t-1","running":true,"active_turn_id":"turn-1","auto_follow_active":false,
                 "approval":{"approval_id":"a-1","server_request_id":"7","summary":"run tests","details":[]}}
            ]}),
            json!({"type":"planning"}),
        ],
    );

    let events = events(client.run_host_cycle(Instant::now(), Duration::ZERO, 16));

    assert_eq!(
        events.len(),
        3,
        "a value the client does not show changes nothing"
    );
    let snapshot = client.snapshot();
    let SessionCatalogSnapshot::Ready(ready) = &snapshot.session_catalog else {
        panic!(
            "expected a ready session list, got {:?}",
            snapshot.session_catalog
        );
    };
    let SessionCatalog::Ready {
        recent_sessions, ..
    } = ready.catalog.as_ref()
    else {
        panic!("expected a ready catalog");
    };
    assert_eq!(recent_sessions.items[0].id, "t-1");
    assert_eq!(recent_sessions.items[0].title(), "Fix CI");
    assert_eq!(snapshot.conversation_runtime.auto_follow.max_auto_turns, 3);
    assert!(
        snapshot
            .conversation_runtime
            .auto_follow
            .continuation_paused
    );
    let Some(AppEvent::ThreadActivityChanged(activity)) = events.last() else {
        panic!("expected thread activity, got {events:?}");
    };
    assert_eq!(activity[0].active_turn_id.as_deref(), Some("turn-1"));
    assert_eq!(
        activity[0]
            .approval
            .as_ref()
            .map(|request| request.summary.as_str()),
        Some("run tests")
    );
}

#[test]
fn startup_waits_for_sync_and_lost_connection_is_not_ready() {
    let (mut client, sender) = super::fixture::client_fixture("/repo", Vec::new());

    let NativeClientDispatchOutcome::Core(started) = client.dispatch_client_event(
        NativeClientEvent::core(CoreInput::Command(AppCommand::RunStartupChecks {
            workspace_directory: "/repo/sub".to_string(),
        })),
    ) else {
        panic!("expected a core outcome");
    };
    assert!(matches!(
        started.events.as_slice(),
        [AppEvent::StartupChanged {
            snapshot: StartupSnapshot::Loading,
            ..
        }]
    ));
    sender
        .send(KernelClientUpdate::Synchronized.into())
        .unwrap();
    client.run_host_cycle(Instant::now(), Duration::ZERO, 16);
    assert!(
        matches!(client.snapshot().startup, StartupSnapshot::Ready(ready)
        if ready.can_continue && !ready.account.ok && ready.attachment.mode_label == "gateway kernel client")
    );

    sender
        .send(KernelClientUpdate::Disconnected("closed".to_string()).into())
        .unwrap();
    client.run_host_cycle(Instant::now(), Duration::ZERO, 16);
    assert!(matches!(
        client.snapshot().session_catalog,
        SessionCatalogSnapshot::Failed { message } if message.contains("reconnecting")
    ));
    assert!(
        matches!(client.snapshot().startup, StartupSnapshot::Failed { message } if message.contains("reconnecting"))
    );
    sender
        .send(KernelClientUpdate::Synchronized.into())
        .unwrap();
    client.run_host_cycle(Instant::now(), Duration::ZERO, 16);
    assert!(matches!(
        client.snapshot().startup,
        StartupSnapshot::Ready(_)
    ));
    assert!(client.is_gateway_kernel_client());
}

#[test]
fn explicit_catalog_load_is_forwarded_to_the_watcher() {
    let (mut client, _sender) = super::fixture::client_fixture("/repo", Vec::new());
    let (refresh, mut requests) = tokio::sync::mpsc::channel(1);
    client.refresh = Some(refresh);
    let _ = client.dispatch_client_event(NativeClientEvent::core(CoreInput::Command(
        AppCommand::LoadSessionCatalog(crate::core::app::SessionCatalogLoadIntent::refresh(
            10, "/repo",
        )),
    )));
    assert!(requests.try_recv().is_ok());
}
