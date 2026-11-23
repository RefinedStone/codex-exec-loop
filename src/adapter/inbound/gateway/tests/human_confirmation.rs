//! Two independent Gateway clients share a kernel; only the main-owned broker can confirm.
use super::*;
use crate::adapter::inbound::desktop_confirmation_bridge::{
    protocol::{FrameReader, write_frame},
    run_io,
};
use crate::composition::repository_kernel::RepositoryKernelRegistry;
use tokio::io::{AsyncWriteExt, ReadHalf, WriteHalf};

mod native_process;

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
async fn criterion_waiver_uses_the_native_broker_and_cannot_survive_acceptance_scope_change() {
    use crate::adapter::inbound::desktop_confirmation_bridge::storage::random_id;

    // Private authority는 workspace 밖에도 남는다. PID가 재사용돼도 이전 멱등 기록과 겹치지 않는다.
    let folder =
        KernelTestFolder::new(&format!("human-criterion-waiver-{}", random_id(8).unwrap()));
    let kernels = Arc::new(RepositoryKernelRegistry::default());
    let mut main = connect_kernel_client(kernels.clone()).await;
    let mut agent = connect_kernel_client(kernels.clone()).await;
    named_attach(&mut main, "attach-main", &folder.workspace()).await;
    named_attach(&mut agent, "attach-agent", &folder.workspace()).await;
    let criterion = json!({"id":"criterion with spaces","scenario":"The expected result exists.","channel":"command","essential":true});
    let created = kernel_request(&mut agent, "create", "akra/kernel/command", json!({
        "command":{"type":"create_task","task":{"title":"Evidence fixture","acceptance":[criterion]}},
        "idempotency_key":"create-evidence-task"
    })).await;
    let task_id = created["result"]["result"]["task_id"]
        .as_str()
        .unwrap_or_else(|| panic!("task creation returns its stable ID: {created}"))
        .to_owned();
    let query = json!({"query":{"type":"task_evidence","task_ids":[task_id]}});
    let state = kernel_request(&mut main, "scope", "akra/kernel/query", query.clone()).await;
    let scope = state["result"]["tasks"][0]["scope"].clone();
    assert!(
        scope["scope_id"].as_str().is_some_and(|id| id.len() == 64),
        "{state}"
    );
    let command = json!({"type":"waive_criterion","request_id":"waiver-one","task_id":task_id,
        "criterion_id":"criterion with spaces","scope_id":scope["scope_id"],
        "expected_acceptance_revision":scope["acceptance_revision"],"reason":"Private waiver reason fixture"});
    let denied = kernel_request(
        &mut agent,
        "raw-waiver",
        "akra/kernel/command",
        json!({
            "command":command,"idempotency_key":"unverified-waiver"
        }),
    )
    .await;
    assert_eq!(
        denied["result"]["rejection"]["reason"], "human_confirmation_required",
        "{denied}"
    );
    let mut helper = MainBroker::start().await;
    let registration = helper.register(&mut main).await;
    let request = json!({"command":command,"idempotency_key":"confirmed-waiver"});
    let prepared = kernel_request(
        &mut main,
        "preview-waiver",
        "akra/kernel/confirmation/prepare",
        json!({"request":request}),
    )
    .await;
    let intent = prepared["result"]["intent"].clone();
    assert_eq!(intent["target"]["kind"], "criterion_waiver", "{prepared}");
    assert_eq!(intent["target"]["scope_id"], scope["scope_id"]);
    assert!(
        prepared["result"]["display"]["detail"]
            .as_str()
            .unwrap()
            .contains("별도로")
    );
    let proof_id = helper.approve(&registration, &intent).await;
    let envelope = json!({"request":request,"confirmation":{
        "registration_id":registration["registration_id"],"proof_id":proof_id,"intent":intent
    }});
    let mut changed = envelope.clone();
    changed["request"]["command"]["reason"] = json!("A different justification");
    let mismatch = kernel_request(
        &mut main,
        "changed-reason",
        "akra/kernel/command_confirmed",
        changed,
    )
    .await;
    assert_eq!(mismatch["error"]["code"], -32602, "{mismatch}");
    let accepted = kernel_request(
        &mut main,
        "waive",
        "akra/kernel/command_confirmed",
        envelope,
    )
    .await;
    assert_eq!(
        accepted["result"]["result"]["type"], "criterion_waived",
        "{accepted}"
    );
    assert_eq!(
        accepted["result"]["result"]["receipt"]["waiver"]["by"],
        "human"
    );
    assert!(
        !accepted["result"]["result"]
            .to_string()
            .contains("Private waiver reason fixture")
    );
    let waived = kernel_request(
        &mut main,
        "waived-state",
        "akra/kernel/query",
        query.clone(),
    )
    .await;
    assert_eq!(
        waived["result"]["tasks"][0]["criteria"][0]["state"], "waived",
        "{waived}"
    );
    let replayed = kernel_request(
        &mut main,
        "waiver-replay",
        "akra/kernel/command",
        request.clone(),
    )
    .await;
    assert_eq!(
        replayed["result"]["operation"]["replayed"], true,
        "{replayed}"
    );

    let mut stale_request = request;
    stale_request["command"]["request_id"] = json!("waiver-two");
    stale_request["idempotency_key"] = json!("stale-waiver");
    let stale_preview = kernel_request(
        &mut main,
        "stale-preview",
        "akra/kernel/confirmation/prepare",
        json!({"request":stale_request}),
    )
    .await;
    let stale_intent = stale_preview["result"]["intent"].clone();
    let stale_proof = helper.approve(&registration, &stale_intent).await;
    let planning = kernel_request(
        &mut agent,
        "current-planning",
        "akra/kernel/query",
        json!({"query":{"type":"planning"}}),
    )
    .await;
    let mut changed_criterion = criterion;
    changed_criterion["scenario"] = json!("A newly agreed result exists.");
    let updated = kernel_request(
        &mut agent,
        "change-criteria",
        "akra/kernel/command",
        json!({
            "command":{"type":"update_task","task_id":task_id,
                "task":{"title":"Evidence fixture","acceptance":[changed_criterion]},
                "expected_planning_revision":planning["result"]["revision"]},
            "idempotency_key":"change-evidence-criteria"
        }),
    )
    .await;
    assert_eq!(
        updated["result"]["result"]["type"], "task_updated",
        "{updated}"
    );
    let stale = kernel_request(&mut main, "consume-stale-waiver", "akra/kernel/command_confirmed", json!({
        "request":stale_request,"confirmation":{"registration_id":registration["registration_id"],"proof_id":stale_proof,"intent":stale_intent}
    })).await;
    assert!(stale["result"]["result"].is_null(), "{stale}");
    let current = kernel_request(&mut main, "new-scope", "akra/kernel/query", query).await;
    assert_ne!(
        current["result"]["tasks"][0]["scope"]["scope_id"],
        scope["scope_id"]
    );
    assert_ne!(
        current["result"]["tasks"][0]["criteria"][0]["state"],
        "waived"
    );
    // 앞선 operation batch를 쓰는 동안 outbox는 비차단으로 미룬다. native 영수증 성공과
    // durable history의 게시 사이에 있는 1초 재시도 경계를 기다린다.
    let waivers = tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            let history = kernel_request(
                &mut main,
                "waiver-history",
                "akra/kernel/query",
                json!({"query":{"type":"history","after":0}}),
            )
            .await;
            let count = history["result"]["records"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|record| record["event"]["kind"] == "criterion_waived")
                .count();
            if count > 0 {
                break count;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("confirmed waiver should reach durable history");
    assert_eq!(
        waivers, 1,
        "one native approval commits one immutable public fact"
    );
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

#[tokio::test(flavor = "multi_thread")]
async fn desktop_autonomy_confirmation_is_scoped_replayable_and_rechecked_after_lowering() {
    let folder = KernelTestFolder::new("human-autonomy");
    let kernels = Arc::new(RepositoryKernelRegistry::default());
    let mut main = connect_kernel_client(kernels.clone()).await;
    let mut agent = connect_kernel_client(kernels.clone()).await;
    named_attach(&mut main, "attach-main", &folder.workspace()).await;
    named_attach(&mut agent, "attach-agent", &folder.workspace()).await;
    let collection_off = kernel_request(
        &mut agent,
        "collection-off",
        "akra/kernel/command",
        json!({"command":{"type":"set_issue_automation_mode","mode":"off"},
            "idempotency_key":"autonomy-collection-off"}),
    )
    .await;
    assert!(
        collection_off["result"]["failure"].is_null(),
        "{collection_off}"
    );

    let command = json!({"type":"set_issue_autonomy","command_id":"autonomy-1","level":"propose"});
    let unverified = kernel_request(
        &mut agent,
        "unverified",
        "akra/kernel/command",
        json!({"command":command,"idempotency_key":"autonomy-unverified"}),
    )
    .await;
    assert_eq!(
        unverified["result"]["rejection"]["reason"], "human_confirmation_required",
        "{unverified}"
    );

    let mut helper = MainBroker::start().await;
    let registration = helper.register(&mut main).await;
    let request = json!({"command":command,"idempotency_key":"autonomy-confirmed"});
    let prepared = kernel_request(
        &mut main,
        "prepare-autonomy",
        "akra/kernel/confirmation/prepare",
        json!({"request":request}),
    )
    .await;
    let intent = prepared["result"]["intent"].clone();
    assert_eq!(intent["target"]["kind"], "autonomy", "{prepared}");
    assert_eq!(intent["target"]["command_id"], "autonomy-1");
    assert_eq!(intent["target"]["from"], "observe");
    assert_eq!(intent["target"]["to"], "propose");
    assert_eq!(intent["target"]["cap"], "propose");
    assert_eq!(intent["target"]["revision"], 0);
    let proof_id = helper.approve(&registration, &intent).await;
    let envelope = json!({"request":request,"confirmation":{
        "registration_id":registration["registration_id"],"proof_id":proof_id,"intent":intent
    }});
    let mut altered = envelope.clone();
    altered["request"]["command"]["command_id"] = json!("another-command");
    let denied = kernel_request(
        &mut main,
        "altered-autonomy",
        "akra/kernel/command_confirmed",
        altered,
    )
    .await;
    assert_eq!(denied["error"]["code"], -32602, "{denied}");

    let accepted = kernel_request(
        &mut main,
        "confirmed-autonomy",
        "akra/kernel/command_confirmed",
        envelope,
    )
    .await;
    assert!(accepted["result"]["rejection"].is_null(), "{accepted}");
    assert!(accepted["result"]["failure"].is_null(), "{accepted}");
    assert_eq!(
        accepted["result"]["result"]["type"],
        "issue_autonomy_changed"
    );
    assert_eq!(accepted["result"]["result"]["command_id"], "autonomy-1");
    assert_eq!(accepted["result"]["result"]["changed"], true);
    assert_eq!(
        accepted["result"]["result"]["autonomy"]["effective_level"],
        "propose"
    );

    let current = kernel_request(
        &mut agent,
        "autonomy-state",
        "akra/kernel/query",
        json!({"query":{"type":"issue_automation"}}),
    )
    .await;
    assert_eq!(
        current["result"]["autonomy"]["requested_level"], "propose",
        "{current}"
    );
    assert_eq!(current["result"]["autonomy"]["effective_level"], "propose");
    assert_eq!(current["result"]["mode"], "off");
    assert_eq!(current["result"]["collecting"], false);

    let replay = kernel_request(&mut main, "autonomy-replay", "akra/kernel/command", request).await;
    assert_eq!(replay["result"]["operation"]["replayed"], true, "{replay}");
    assert_eq!(
        replay["result"]["operation"]["op_id"],
        accepted["result"]["operation"]["op_id"]
    );

    let stale_request = json!({
        "command":{"type":"set_issue_autonomy","command_id":"autonomy-stale","level":"propose"},
        "idempotency_key":"autonomy-stale"
    });
    let stale_preview = kernel_request(
        &mut main,
        "prepare-stale",
        "akra/kernel/confirmation/prepare",
        json!({"request":stale_request}),
    )
    .await;
    let stale_intent = stale_preview["result"]["intent"].clone();
    assert_eq!(stale_intent["target"]["revision"], 1, "{stale_preview}");
    let stale_proof = helper.approve(&registration, &stale_intent).await;
    let lowered = kernel_request(
        &mut agent,
        "lower-autonomy",
        "akra/kernel/command",
        json!({"command":{"type":"set_issue_autonomy","command_id":"autonomy-off","level":"off"},
            "idempotency_key":"autonomy-lower"}),
    )
    .await;
    assert!(lowered["result"]["rejection"].is_null(), "{lowered}");
    assert!(lowered["result"]["failure"].is_null(), "{lowered}");
    assert_eq!(
        lowered["result"]["result"]["autonomy"]["effective_level"],
        "off"
    );

    let stale = kernel_request(
        &mut main,
        "stale-confirmed",
        "akra/kernel/command_confirmed",
        json!({"request":stale_request,"confirmation":{
            "registration_id":registration["registration_id"],
            "proof_id":stale_proof,"intent":stale_intent
        }}),
    )
    .await;
    assert_eq!(
        stale["result"]["rejection"]["reason"], "human_confirmation_required",
        "{stale}"
    );
    let final_state = kernel_request(
        &mut agent,
        "autonomy-final",
        "akra/kernel/query",
        json!({"query":{"type":"issue_automation"}}),
    )
    .await;
    assert_eq!(
        final_state["result"]["autonomy"]["effective_level"], "off",
        "{final_state}"
    );
    assert_eq!(final_state["result"]["autonomy"]["revision"], 2);
    detach(&mut main).await;
    detach(&mut agent).await;
    helper.stop().await;
    kernels.shutdown();
}
