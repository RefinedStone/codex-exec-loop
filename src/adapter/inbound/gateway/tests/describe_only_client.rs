//! A real kernel behind the Gateway with a fake app-server. No model turn runs (#2804, G2).

use super::*;

use crate::composition::native_client_runtime::tests::{
    RecordedTurns, TurnGate, recording_runtime_with_turn_gate_for_test,
};
use crate::composition::repository_kernel::RepositoryKernelRegistry;

/// Every new kernel gets a runtime whose app-server lists `thread-2` and whose turns wait for the gate.
fn kernels_with_fake_app_server(
    gate: TurnGate,
    turns: Arc<Mutex<Vec<RecordedTurns>>>,
) -> Arc<RepositoryKernelRegistry> {
    Arc::new(RepositoryKernelRegistry::with_attach_runtime_for_test(
        move |_workspace, issues, admission| {
            let (mut runtime, recorded) = recording_runtime_with_turn_gate_for_test(gate.clone());
            turns.lock().unwrap().push(recorded);
            runtime.attach_issue_binding_pause(
                issues
                    .binding_pause_service()
                    .expect("the kernel test collection has a binding pause"),
                admission,
            )?;
            Ok(runtime)
        },
    ))
}

async fn attach_transient(client: &mut WebSocketStream<DuplexStream>, workspace: &str) -> String {
    let attached = kernel_request(
        client,
        "attach",
        "akra/kernel/attach",
        json!({"workspace": workspace, "client_name": "akra-cli", "transient": true}),
    )
    .await;
    attached["result"]["kernel"]
        .as_str()
        .unwrap_or_else(|| panic!("{attached}"))
        .to_string()
}

async fn wait_until(condition: impl Fn() -> bool) {
    tokio::time::timeout(TEST_TIMEOUT, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the condition should hold in time");
}

/*
 * The client reads only `describe`. It finds the thread prompt command and the operation query by their input
 * fields, follows `next` of a rejection, and polls the operation until its state is an end. It names no command
 * and no reason code. `waiting` runs when the client starts to wait for an event.
 */
async fn describe_only_client(
    client: &mut WebSocketStream<DuplexStream>,
    workspace: &str,
    thread_id: &str,
    mut waiting: impl FnMut(),
) -> Value {
    let spec = kernel_request(client, "describe", "akra/kernel/describe", json!({})).await;
    let spec = &spec["result"];
    let command = spec["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            let properties = &entry["input"]["properties"];
            properties.get("prompt").is_some()
                && properties.get("thread_id").is_some()
                && properties.get("expected_turn_id").is_none()
        })
        .expect("describe names one thread prompt command");
    let query = spec["queries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["input"]["properties"].get("op_id").is_some())
        .expect("describe names the operation query");
    let attached = kernel_request(
        client,
        "describe-attach",
        "akra/kernel/attach",
        json!({"workspace": workspace, "client_name": "spec-only-client", "transient": true}),
    )
    .await;
    assert!(attached["result"]["kernel"].is_string(), "{attached}");

    let input = json!({"type": command["name"], "thread_id": thread_id, "prompt": "a prompt"});
    let mut rejected = 0;
    let op_id = loop {
        let id = format!("prompt-{rejected}");
        let answer = kernel_request(
            client,
            &id,
            "akra/kernel/command",
            json!({"command": input, "idempotency_key": id}),
        )
        .await;
        let answer = &answer["result"];
        let Some(next) = answer["rejection"]["next"].as_array() else {
            break answer["operation"]["op_id"].clone();
        };
        rejected += 1;
        assert!(rejected < 5, "the client keeps being rejected: {answer}");
        let mut resend = false;
        for step in next {
            match step["step"].as_str().unwrap() {
                "wait_for" => {
                    waiting();
                    next_json_matching(client, |value| {
                        value["params"]["event"]["type"] == step["event"]
                    })
                    .await;
                }
                "resend" => resend = true,
                other => panic!("the client cannot follow `{other}`: {answer}"),
            }
        }
        assert!(resend, "{answer}");
    };
    assert!(
        rejected > 0,
        "the kernel was busy, so the client followed `next`"
    );

    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            let operation = kernel_request(
                client,
                "operation",
                "akra/kernel/query",
                json!({"query": {"type": query["name"], "op_id": op_id}}),
            )
            .await;
            let state = operation["result"]["state"].as_str().unwrap().to_string();
            if !["accepted", "running"].contains(&state.as_str()) {
                return operation["result"].clone();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the operation should end")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cli_prompt_reaches_a_thread_after_the_kernel_lost_its_session_list() {
    let folder = KernelTestFolder::new("unknown-thread");
    let workspace = folder.workspace();
    let gate = TurnGate::closed();
    let turns = Arc::new(Mutex::new(Vec::new()));
    let kernels = kernels_with_fake_app_server(gate.clone(), turns.clone());
    let idle = super::super::register_idle_notification(&kernels);

    // 1. The CLI loads the session list and leaves. Its kernel goes down and forgets the list.
    let mut cli = connect_kernel_client(kernels.clone()).await;
    let first_kernel = attach_transient(&mut cli, &workspace).await;
    let loaded = kernel_request(
        &mut cli,
        "load",
        "akra/kernel/command",
        json!({"command": {"type": "load_sessions"}}),
    )
    .await;
    assert!(loaded["result"].get("rejection").is_none(), "{loaded}");
    let unloaded = idle.0.notified();
    tokio::pin!(unloaded);
    unloaded.as_mut().enable();
    cli.close(None).await.expect("close");
    tokio::time::timeout(TEST_TIMEOUT, unloaded)
        .await
        .expect("the transient close wakes the idle reaper");
    wait_until(|| kernels.unload_idle_kernels(std::time::Instant::now()) == [first_kernel.clone()])
        .await;

    // 2. The CLI sends a prompt to a new kernel. The kernel asks the app-server about the thread and accepts it.
    let mut cli = connect_kernel_client(kernels.clone()).await;
    attach_transient(&mut cli, &workspace).await;
    let submitted = kernel_request(
        &mut cli,
        "submit",
        "akra/kernel/command",
        json!({
            "command": {"type": "submit_prompt", "thread_id": "thread-2", "prompt": "fix the test"},
            "idempotency_key": "cli-submit",
        }),
    )
    .await;
    assert!(
        submitted["result"].get("rejection").is_none(),
        "{submitted}"
    );
    assert_eq!(
        submitted["result"]["operation"]["state"],
        json!("accepted"),
        "{submitted}"
    );
    cli.close(None).await.expect("close");
    let second_turns = turns.lock().unwrap()[1].clone();
    wait_until(|| second_turns.lock().unwrap().len() == 1).await;
    assert_eq!(second_turns.lock().unwrap()[0].0, "thread-2");
    assert!(
        kernels
            .unload_idle_kernels(std::time::Instant::now())
            .is_empty(),
        "the turn of the CLI prompt keeps the kernel up"
    );

    // 3. G2: a client that reads only `describe` prompts the busy kernel, follows `next`, and sees the end.
    let mut client = connect_kernel_client(kernels.clone()).await;
    let operation = describe_only_client(&mut client, &workspace, "thread-2", || gate.open()).await;
    assert_eq!(operation["state"], json!("failed"), "{operation}");
    assert_eq!(second_turns.lock().unwrap().len(), 2);
}
