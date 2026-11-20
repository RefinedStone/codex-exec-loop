//! Two independent Gateway clients share a kernel; only the main-owned broker can confirm.
use super::*;
use crate::adapter::inbound::desktop_confirmation_bridge::{
    protocol::{FrameReader, write_frame},
    run_io,
};
use crate::composition::repository_kernel::RepositoryKernelRegistry;
use tokio::io::{AsyncWriteExt, ReadHalf, WriteHalf};

struct MainBroker {
    input: WriteHalf<DuplexStream>,
    output: FrameReader<ReadHalf<DuplexStream>>,
    task: tokio::task::JoinHandle<Result<()>>,
}
impl MainBroker {
    async fn start() -> Self {
        let run_dir = crate::configuration::akra_home_dir()
            .unwrap()
            .join(crate::gateway_endpoint::RUN_DIRECTORY_NAME);
        let (main, helper) = tokio::io::duplex(128 * 1024);
        let (input, output) = tokio::io::split(helper);
        let task = tokio::spawn(async move { run_io(&run_dir, input, output).await });
        let (output, input) = tokio::io::split(main);
        let mut helper = Self {
            input,
            output: FrameReader::new(output),
            task,
        };
        assert_eq!(helper.next().await, json!({"type":"ready"}));
        helper
    }
    async fn next(&mut self) -> Value {
        tokio::time::timeout(TEST_TIMEOUT, self.output.read())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    }
    async fn request(&mut self, value: Value) -> Value {
        write_frame(&mut self.input, &value).await.unwrap();
        self.next().await
    }
    async fn register(&mut self, main: &mut WebSocketStream<DuplexStream>) -> Value {
        let prepared = self
            .request(json!({"id":"prepare","command":"prepare"}))
            .await;
        let registration = prepared["result"].clone();
        let registered = kernel_request(
            main,
            "register",
            "akra/kernel/confirmation/register",
            registration.clone(),
        )
        .await;
        assert!(registered["result"]["binding"].is_object(), "{registered}");
        registration
    }
    async fn approve(&mut self, registration: &Value, intent: &Value) -> String {
        let approved = self
            .request(json!({"id":"approve","command":"approve",
            "registration_id":registration["registration_id"],"intent":intent}))
            .await;
        approved["result"]["proof_id"]
            .as_str()
            .expect("main approval produces proof")
            .to_owned()
    }
    async fn stop(mut self) {
        self.input.shutdown().await.unwrap();
        tokio::time::timeout(TEST_TIMEOUT, self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

async fn named_attach(client: &mut WebSocketStream<DuplexStream>, id: &str, workspace: &str) {
    let attached = kernel_request(
        client,
        id,
        "akra/kernel/attach",
        json!({"workspace":workspace,"client_name":"Desktop"}),
    )
    .await;
    assert!(attached["result"]["client_id"].is_string(), "{attached}");
}
async fn detach(client: &mut WebSocketStream<DuplexStream>) {
    kernel_request(client, "detach", "akra/kernel/detach", json!({})).await;
    client.close(None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn desktop_name_is_not_proof_and_an_exact_confirmed_mode_command_replays_without_proof() {
    let folder = KernelTestFolder::new("human-mode");
    let kernels = Arc::new(RepositoryKernelRegistry::default());
    let mut main = connect_kernel_client(kernels.clone()).await;
    let mut agent = connect_kernel_client(kernels.clone()).await;
    named_attach(&mut main, "attach-main", &folder.workspace()).await;
    named_attach(&mut agent, "attach-agent", &folder.workspace()).await;
    let initial = kernel_request(&mut agent,"initial-off","akra/kernel/command",
        json!({"command":{"type":"set_issue_automation_mode","mode":"off"},"idempotency_key":"initial-off"})).await;
    assert!(initial["result"]["failure"].is_null(), "{initial}");
    let command = json!({"type":"set_issue_automation_mode","mode":"observe"});
    let unverified = kernel_request(
        &mut agent,
        "unverified",
        "akra/kernel/command",
        json!({"command":command,"idempotency_key":"unverified-up"}),
    )
    .await;
    assert_eq!(
        unverified["result"]["rejection"]["reason"], "human_confirmation_required",
        "{unverified}"
    );

    let mut helper = MainBroker::start().await;
    let registration = helper.register(&mut main).await;
    let stolen_registration = kernel_request(
        &mut agent,
        "stolen-register",
        "akra/kernel/confirmation/register",
        registration.clone(),
    )
    .await;
    assert_eq!(
        stolen_registration["error"]["code"], -32602,
        "{stolen_registration}"
    );
    let request = json!({"command":command,"idempotency_key":"confirmed-mode-up"});
    let prepared = kernel_request(
        &mut main,
        "preview",
        "akra/kernel/confirmation/prepare",
        json!({"request":request}),
    )
    .await;
    let intent = prepared["result"]["intent"].clone();
    assert_eq!(intent["target"]["kind"], "mode", "{prepared}");
    assert_eq!(intent["target"]["to"], "observe");
    assert!(prepared["result"]["display"]["confirm_label"].is_string());
    let proof_id = helper.approve(&registration, &intent).await;
    let confirmation = json!({"registration_id":registration["registration_id"],"proof_id":proof_id,"intent":intent});
    let envelope = json!({"request":request,"confirmation":confirmation});
    let stolen = kernel_request(
        &mut agent,
        "stolen",
        "akra/kernel/command_confirmed",
        envelope.clone(),
    )
    .await;
    assert_eq!(stolen["error"]["code"], -32602, "{stolen}");
    let mut changed = envelope.clone();
    changed["request"]["idempotency_key"] = json!("changed-key");
    let mismatch = kernel_request(
        &mut main,
        "mismatch",
        "akra/kernel/command_confirmed",
        changed,
    )
    .await;
    assert_eq!(mismatch["error"]["code"], -32602, "{mismatch}");

    let accepted = kernel_request(
        &mut main,
        "accepted",
        "akra/kernel/command_confirmed",
        envelope.clone(),
    )
    .await;
    assert_eq!(accepted["result"]["processed"], true, "{accepted}");
    assert!(accepted["result"]["rejection"].is_null(), "{accepted}");
    assert!(accepted["result"]["failure"].is_null(), "{accepted}");
    assert!(
        accepted["result"]["operation"]["op_id"].is_i64(),
        "{accepted}"
    );
    let replay = kernel_request(&mut main, "replay", "akra/kernel/command", request.clone()).await;
    assert_eq!(replay["result"]["operation"]["replayed"], true, "{replay}");
    assert_eq!(
        replay["result"]["operation"]["op_id"],
        accepted["result"]["operation"]["op_id"]
    );
    let retried = kernel_request(
        &mut main,
        "exact-retry",
        "akra/kernel/command_confirmed",
        envelope.clone(),
    )
    .await;
    assert_eq!(
        retried["result"]["operation"]["replayed"], true,
        "{retried}"
    );
    let lowered = kernel_request(&mut agent,"lower","akra/kernel/command",
        json!({"command":{"type":"set_issue_automation_mode","mode":"off"},"idempotency_key":"lower-mode"})).await;
    assert!(lowered["result"]["rejection"].is_null(), "{lowered}");
    assert!(lowered["result"]["failure"].is_null(), "{lowered}");
    assert_eq!(lowered["result"]["result"]["mode"], "off", "{lowered}");
    let mut reused = envelope;
    reused["request"]["idempotency_key"] = json!("reused-proof-new-command");
    reused["confirmation"]["intent"]["key"] = json!("reused-proof-new-command");
    let denied = kernel_request(&mut main, "reuse", "akra/kernel/command_confirmed", reused).await;
    assert_eq!(denied["error"]["code"], -32602, "{denied}");
    detach(&mut main).await;
    detach(&mut agent).await;
    helper.stop().await;
    kernels.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn confirmation_inputs_errors_and_malformed_frames_are_never_recorded_or_forwarded() {
    const SECRET: &str = "human-confirmation-private-canary";
    let folder = KernelTestFolder::new("human-recording");
    let recordings = folder.path.join("recordings");
    let recorder = Arc::new(super::super::recorder::GatewayRecorder::create(&recordings).unwrap());
    let kernels = Arc::new(RepositoryKernelRegistry::default());
    let session_kernels = kernels.clone();
    let (client_side, gateway_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
    let session = tokio::spawn(async move {
        run_session(
            gateway_side,
            GatewayHosts::new(EchoUpstream::new(true), Vec::new()),
            session_kernels,
            super::super::session::GatewayServices::default(),
            Some(recorder),
        )
        .await
    });
    let (mut client, _) = tokio_tungstenite::client_async_with_config(
        "ws://localhost/",
        client_side,
        Some(websocket_config()),
    )
    .await
    .unwrap();
    for (index, method) in [
        "akra/kernel/confirmation/register",
        "akra/kernel/confirmation/prepare",
        "akra/kernel/command_confirmed",
    ]
    .into_iter()
    .enumerate()
    {
        let reply = kernel_request(
            &mut client,
            &format!("invalid-{index}"),
            method,
            json!({
                "registration_id":SECRET,"session_file":SECRET,"proof_id":SECRET,"unknown":SECRET
            }),
        )
        .await;
        assert_eq!(reply["error"]["code"], -32602, "{reply}");
        assert!(!reply.to_string().contains(SECRET));
    }
    let malformed = format!(
        r#"{{"jsonrpc":"2.0","method":"akra/kernel/command_confirmed","params":{{"proof_id":"{SECRET}""#
    );
    client.send(Message::text(malformed)).await.unwrap();
    let rejected = next_json_matching(&mut client, |value| value["error"]["code"] == -32602).await;
    assert!(!rejected.to_string().contains(SECRET));

    client
        .send(Message::text(
            json!([{
                "jsonrpc":"2.0","id":"unsupported-batch","method":"akra/kernel/command_confirmed",
                "params":{"confirmation":{"proof_id":SECRET}}
            }])
            .to_string(),
        ))
        .await
        .unwrap();
    let batch_rejected =
        next_json_matching(&mut client, |value| value["error"]["code"] == -32602).await;
    assert!(!batch_rejected.to_string().contains(SECRET));

    for escaped_method in [
        r"akra\/kernel\/command_confirmed",
        r"akra\u002fkernel\u002fcommand_confirmed",
    ] {
        let malformed = format!(
            r#"{{"jsonrpc":"2.0","method":"{escaped_method}","params":{{"proof_id":"{SECRET}""#
        );
        client.send(Message::text(malformed)).await.unwrap();
        let rejected =
            next_json_matching(&mut client, |value| value["error"]["code"] == -32602).await;
        assert!(!rejected.to_string().contains(SECRET));
    }
    let duplicate = format!(
        r#"{{"method":"akra/kernel/command_confirmed","method":"turn/start","params":{{"proof_id":"{SECRET}"}}}}"#
    );
    client.send(Message::text(duplicate)).await.unwrap();
    let duplicate_rejected =
        next_json_matching(&mut client, |value| value["error"]["code"] == -32602).await;
    assert!(!duplicate_rejected.to_string().contains(SECRET));

    named_attach(&mut client, "attach", &folder.workspace()).await;
    let initial = kernel_request(&mut client,"initial-off","akra/kernel/command",
        json!({"command":{"type":"set_issue_automation_mode","mode":"off"},"idempotency_key":"recording-initial-off"})).await;
    assert!(initial["result"]["failure"].is_null(), "{initial}");
    let mut helper = MainBroker::start().await;
    let registration = helper.register(&mut client).await;
    let session_file = registration["session_file"].as_str().unwrap();
    let credential: Value = serde_json::from_slice(&std::fs::read(session_file).unwrap()).unwrap();
    let request = json!({"command":{"type":"set_issue_automation_mode","mode":"observe"},"idempotency_key":"recording-up"});
    let prepared = kernel_request(
        &mut client,
        "preview",
        "akra/kernel/confirmation/prepare",
        json!({"request":request}),
    )
    .await;
    let intent = prepared["result"]["intent"].clone();
    let proof_id = helper.approve(&registration, &intent).await;
    let sent = kernel_request(&mut client,"approved","akra/kernel/command_confirmed",json!({
        "request":request,"confirmation":{"registration_id":registration["registration_id"],"proof_id":proof_id,"intent":intent}
    })).await;
    assert_eq!(sent["result"]["processed"], true, "{sent}");
    kernel_request(&mut client, "marker", "akra/gateway/info", json!({})).await;
    let recorded: String = std::fs::read_dir(&recordings)
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect();
    assert!(recorded.contains("akra/gateway/info"));
    for forbidden in [
        SECRET,
        session_file,
        credential["token"].as_str().unwrap(),
        proof_id.as_str(),
    ] {
        assert!(
            !recorded.contains(forbidden),
            "secret confirmation material was recorded"
        );
        assert!(
            !sent.to_string().contains(forbidden),
            "secret confirmation material was returned"
        );
    }
    detach(&mut client).await;
    helper.stop().await;
    tokio::time::timeout(TEST_TIMEOUT, session)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    kernels.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn only_main_proof_records_a_human_decision_and_later_agent_answers_preserve_it() {
    let folder = KernelTestFolder::new("human-decision");
    let workspace = crate::composition::production::repository_kernel_target(&folder.workspace())
        .unwrap()
        .workspace_root;
    let (decision_id, option) =
        crate::application::service::issue_decision::seed_human_decision_for_gateway(&workspace);
    let kernels = Arc::new(RepositoryKernelRegistry::default());
    let mut main = connect_kernel_client(kernels.clone()).await;
    let mut agent = connect_kernel_client(kernels.clone()).await;
    named_attach(&mut main, "attach-main", &folder.workspace()).await;
    named_attach(&mut agent, "attach-agent", &folder.workspace()).await;
    let command = json!({"type":"answer_issue_decision","decision_id":decision_id,"option":option});
    let rejected = kernel_request(
        &mut agent,
        "self-confirmation",
        "akra/kernel/command",
        json!({"command":command,"idempotency_key":"agent-answer"}),
    )
    .await;
    assert_eq!(
        rejected["result"]["rejection"]["reason"], "human_confirmation_required",
        "{rejected}"
    );
    let mut helper = MainBroker::start().await;
    let registration = helper.register(&mut main).await;
    let request = json!({"command":command,"idempotency_key":"main-human-answer"});
    let prepared = kernel_request(
        &mut main,
        "preview",
        "akra/kernel/confirmation/prepare",
        json!({"request":request}),
    )
    .await;
    let intent = prepared["result"]["intent"].clone();
    assert_eq!(intent["target"]["decision_id"], decision_id, "{prepared}");
    assert_eq!(intent["target"]["option"], option);
    let proof_id = helper.approve(&registration, &intent).await;
    let envelope = json!({"request":request,"confirmation":{
        "registration_id":registration["registration_id"],"proof_id":proof_id,"intent":intent
    }});
    let stolen = kernel_request(
        &mut agent,
        "forged-client",
        "akra/kernel/command_confirmed",
        envelope.clone(),
    )
    .await;
    assert_eq!(stolen["error"]["code"], -32602, "{stolen}");
    let mut another_decision = envelope.clone();
    another_decision["request"]["command"]["decision_id"] = json!("other-decision");
    let denied = kernel_request(
        &mut main,
        "other-decision",
        "akra/kernel/command_confirmed",
        another_decision,
    )
    .await;
    assert_eq!(denied["error"]["code"], -32602, "{denied}");
    let answered = kernel_request(
        &mut main,
        "human-answer",
        "akra/kernel/command_confirmed",
        envelope,
    )
    .await;
    assert_eq!(
        answered["result"]["result"]["type"], "issue_decision_resolved",
        "{answered}"
    );
    assert_eq!(
        answered["result"]["result"]["decision"]["answer_by"], "human",
        "{answered}"
    );
    assert_eq!(
        answered["result"]["result"]["decision"]["state"],
        "answered"
    );
    assert_eq!(answered["result"]["result"]["changed"], true);
    let repeated = kernel_request(
        &mut agent,
        "repeat-without-proof",
        "akra/kernel/command",
        request,
    )
    .await;
    assert_eq!(
        repeated["result"]["operation"]["replayed"], true,
        "{repeated}"
    );
    assert_eq!(
        repeated["result"]["operation"]["op_id"],
        answered["result"]["operation"]["op_id"]
    );
    let later = kernel_request(
        &mut agent,
        "later-agent",
        "akra/kernel/command",
        json!({"command":command,"idempotency_key":"later-agent-answer"}),
    )
    .await;
    assert_eq!(
        later["result"]["result"]["decision"]["answer_by"], "human",
        "{later}"
    );
    assert_eq!(later["result"]["result"]["changed"], false);
    for output in [&prepared, &stolen, &answered, &repeated, &later] {
        let text = output.to_string();
        assert!(!text.contains(&proof_id));
        assert!(!text.contains("PRIVATE_GATEWAY_ORIGINAL_CANARY"));
    }
    detach(&mut main).await;
    detach(&mut agent).await;
    helper.stop().await;
    kernels.shutdown();
}
