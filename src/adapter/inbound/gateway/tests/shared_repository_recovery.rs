//! #2800: real Gateway sessions, writer lock and journal, with a fake app-server runtime.
//! Desktop is represented at the RPC boundary. CLI and TUI share the production request transport;
//! this fixture supplies their wire payloads, not their user interfaces.

use super::*;
use crate::adapter::inbound::kernel_protocol::ERROR_IDEMPOTENCY_KEY_REUSED;
use crate::adapter::outbound::filesystem::{RepositoryKernelLockHeld, RepositoryKernelOwner};
use crate::adapter::outbound::gateway_kernel_client::request_over_socket;
use crate::application::port::outbound::kernel_gateway_port::KernelGatewayReply;
use crate::composition::native_client_runtime::tests::recording_runtime_for_test;
use crate::composition::{production, repository_kernel::RepositoryKernelRegistry};
use crate::domain::kernel_event::{KernelEvent, MAX_KERNEL_EVENT_PAGE};

const KEY: &str = "shared-repository-recovery-key";
const COMMAND: &str = "akra/kernel/command";
const QUERY: &str = "akra/kernel/query";
const DESKTOP: &str = "akra-desktop";

#[tokio::test(flavor = "multi_thread")]
async fn three_client_wire_paths_share_a_receipt_and_recover_the_history_cursor() {
    tokio::time::timeout(TEST_TIMEOUT, shared_repository_round_trip())
        .await
        .expect("shared repository recovery should finish");
}

async fn shared_repository_round_trip() {
    let folder = KernelTestFolder::new("shared-repository-recovery");
    let git = crate::test_utils::git_command()
        .args(["init", "--quiet"])
        .current_dir(&folder.path)
        .output()
        .expect("git should start");
    assert!(git.status.success(), "{git:?}");
    let workspace = folder.workspace();
    let turns = Arc::new(Mutex::new(None));
    let runtime_turns = turns.clone();
    let kernels = Arc::new(RepositoryKernelRegistry::with_attach_runtime_for_test(
        move |_, issues, admission| {
            let (mut runtime, recorded) = recording_runtime_for_test();
            assert!(runtime_turns.lock().unwrap().replace(recorded).is_none());
            runtime.attach_issue_binding_pause(
                issues
                    .binding_pause_service()
                    .expect("binding pause service"),
                admission,
            )?;
            Ok(runtime)
        },
    ));

    let mut desktop = connect_kernel_client(kernels.clone()).await;
    let desktop_kernel = attach(&mut desktop, &workspace, DESKTOP).await;
    let mut observer = connect_kernel_client(kernels.clone()).await;
    assert_eq!(
        attach(&mut observer, &workspace, "shared-repository-observer").await,
        desktop_kernel
    );
    assert_gateway_owns_writer_lock(&workspace);

    let baseline = history(&mut desktop, 0, MAX_KERNEL_EVENT_PAGE).await;
    let saved_cursor = baseline["next_after"].as_i64().unwrap();
    let request = command(3);
    desktop
        .send(Message::text(
            json!({"jsonrpc":"2.0","id":"lost","method":COMMAND,"params":request}).to_string(),
        ))
        .await
        .unwrap();
    // Do not read any answer on this socket. A separate subscriber proves execution before
    // we drop it, so this is an unknown outcome, not a request lost before dispatch.
    let change = next_json_matching(&mut observer, |message| {
        let event = &message["params"]["event"];
        event["type"] == "recorded"
            && event["event"]["kind"] == "auto_follow_configured"
            && event["event"]["max_auto_turns"] == 3
    })
    .await;
    let mut change = change["params"]["event"].clone();
    change.as_object_mut().unwrap().remove("type");
    let change_sequence = change["sequence"].as_i64().unwrap();
    drop(desktop);

    let mut recovered = connect_kernel_client(kernels.clone()).await;
    assert_eq!(
        attach(&mut recovered, &workspace, DESKTOP).await,
        desktop_kernel
    );
    // Recover with read-only history before the client makes any explicit replay.
    let received = history(&mut recovered, saved_cursor, 1).await;
    let op_id = received["next_after"].as_i64().unwrap();
    assert!(saved_cursor < op_id && op_id < change_sequence);
    assert_eq!(received["records"].as_array().unwrap().len(), 1);
    assert_eq!(received["records"][0]["sequence"], op_id);
    assert_eq!(received["records"][0]["event"]["op_id"], op_id);
    assert_eq!(received["records"][0]["event"]["kind"], "command_received");
    assert_eq!(received["records"][0]["event"]["caller"], DESKTOP);
    assert_eq!(received["records"][0]["event"]["idempotency_key"], KEY);
    let applied = history(&mut recovered, op_id, 1).await;
    assert_eq!(applied["next_after"], change_sequence);
    assert_eq!(applied["records"].as_array().unwrap().len(), 1);
    assert_eq!(applied["records"][0], change);

    for client_name in ["akra-cli", "akra-tui"] {
        let answer =
            native_request(&kernels, &workspace, client_name, COMMAND, request.clone()).await;
        assert_replay(&answer, op_id);
        let page = native_request(
            &kernels,
            &workspace,
            client_name,
            QUERY,
            history_query(op_id, 1),
        )
        .await;
        assert_eq!(page["next_after"], applied["next_after"]);
        assert_eq!(page["records"], applied["records"]);
    }
    let replay = kernel_request(&mut recovered, "replay", COMMAND, request).await;
    assert_replay(&replay["result"], op_id);
    let conflict = kernel_request(&mut recovered, "conflict", COMMAND, command(4)).await;
    assert_eq!(conflict["error"]["code"], ERROR_IDEMPOTENCY_KEY_REUSED);

    let tail = history(&mut recovered, change_sequence, MAX_KERNEL_EVENT_PAGE).await;
    assert_eq!(tail["records"], json!([]));
    assert_eq!(tail["next_after"], change_sequence);
    let live = kernel_request(
        &mut observer,
        "read",
        QUERY,
        json!({"query":{"type":"auto_follow"}}),
    )
    .await;
    assert_eq!(live["result"]["max_turns"], 3);

    assert_gateway_owns_writer_lock(&workspace);
    drop(recovered);
    drop(observer);
    kernels.shutdown();
    let _released = production::acquire_repository_kernel_lock(&workspace)
        .expect("shutdown should release the disposable writer lock");

    let records = production::kernel_event_log_port()
        .load_kernel_events_after(&workspace, saved_cursor, MAX_KERNEL_EVENT_PAGE)
        .unwrap()
        .records;
    assert_eq!(records.len(), 2, "{records:?}");
    assert_eq!(records[0].sequence, op_id);
    assert!(matches!(
        &records[0].event,
        KernelEvent::CommandReceived { idempotency_key: Some(key), .. } if key == KEY
    ));
    assert_eq!(records[1].sequence, change_sequence);
    assert!(matches!(
        &records[1].event,
        KernelEvent::AutoFollowConfigured {
            max_auto_turns: 3,
            ..
        }
    ));
    let turns = turns.lock().unwrap().take().unwrap();
    assert!(turns.lock().unwrap().is_empty(), "no app-server turn");
    eprintln!("shared recovery: cursor {saved_cursor}, op_id {op_id}, tail {change_sequence}");
}

fn assert_gateway_owns_writer_lock(workspace: &str) {
    let error = match production::acquire_repository_kernel_lock(workspace) {
        Ok(_) => panic!("an attached repository kernel must exclude a second writer"),
        Err(error) => error,
    };
    let held = error
        .downcast_ref::<RepositoryKernelLockHeld>()
        .unwrap_or_else(|| panic!("expected writer lock contention: {error:#}"));
    assert_eq!(held.owner, Some(RepositoryKernelOwner::Gateway));
}

async fn attach(client: &mut WebSocketStream<DuplexStream>, workspace: &str, name: &str) -> Value {
    let reply = kernel_request(
        client,
        "attach",
        "akra/kernel/attach",
        json!({"workspace":workspace,"client_name":name}),
    )
    .await;
    let id = &reply["result"]["kernel"];
    assert!(id.is_string(), "{reply}");
    id.clone()
}

fn command(value: u32) -> Value {
    json!({
        "command":{"type":"set_auto_follow_max_turns","value":value},
        "idempotency_key":KEY
    })
}

async fn history(client: &mut WebSocketStream<DuplexStream>, after: i64, limit: usize) -> Value {
    let reply = kernel_request(client, "history", QUERY, history_query(after, limit)).await;
    assert_eq!(reply["result"]["type"], "history", "{reply}");
    reply["result"].clone()
}

fn history_query(after: i64, limit: usize) -> Value {
    json!({"query":{"type":"history","after":after,"limit":limit}})
}

async fn native_request(
    kernels: &Arc<RepositoryKernelRegistry>,
    workspace: &str,
    client_name: &str,
    method: &str,
    params: Value,
) -> Value {
    let mut socket = connect_kernel_client(kernels.clone()).await;
    let reply = request_over_socket(&mut socket, workspace, client_name, method, params, false)
        .await
        .unwrap();
    let KernelGatewayReply::Answer(answer) = reply else {
        panic!("{client_name} did not receive a Gateway answer for {method}");
    };
    answer
}

fn assert_replay(answer: &Value, op_id: i64) {
    assert_eq!(answer["operation"]["op_id"], op_id, "{answer}");
    assert_eq!(answer["operation"]["replayed"], true, "{answer}");
}
