//! 실제 WebSocket·녹화 경계를 통해 결정 답의 원문이 남지 않는지 확인한다.
use super::*;
use crate::composition::repository_kernel::RepositoryKernelRegistry;

#[tokio::test(flavor = "multi_thread")]
async fn issue_decision_notes_and_decoder_errors_stay_out_of_gateway_recordings() {
    const SECRET: &str = "decision-private-note-canary-2544";
    let folder = KernelTestFolder::new("decision-privacy");
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
    let command = json!({
        "type": "answer_issue_decision", "decision_id": "not-present",
        "option": "continue", "note": SECRET
    });
    // Attach 이전에 거절된 요청도 로컬 원문을 녹화하지 않는다.
    let before = kernel_request(
        &mut client,
        "before",
        "akra/kernel/command",
        json!({"command": command.clone()}),
    )
    .await;
    assert_eq!(before["error"]["code"], -32011);
    assert!(!before.to_string().contains(SECRET));

    attach_kernel(&mut client, "attach", &folder.workspace()).await;

    // serde의 unknown-field 오류에는 필드 이름이 들어갈 수 있다. 결정 답은 고정 문구로 응답한다.
    let mut malformed = command.clone();
    malformed[SECRET] = json!("private field");
    let rejected = kernel_request(
        &mut client,
        "malformed",
        "akra/kernel/command",
        json!({"command": malformed}),
    )
    .await;
    assert_eq!(rejected["error"]["code"], -32602);
    assert_eq!(
        rejected["error"]["message"],
        "invalid issue decision answer parameters"
    );
    assert!(!rejected.to_string().contains(SECRET));

    let mut forged = command.clone();
    forged["by"] = json!("human");
    let rejected = kernel_request(
        &mut client,
        "forged",
        "akra/kernel/command",
        json!({"command": forged}),
    )
    .await;
    assert_eq!(rejected["error"]["code"], -32602);
    assert!(!rejected.to_string().contains(SECRET));

    // 존재하지 않는 요청은 실제 저장 명령 경로에서 실패한다. 실패 영수증에도 note는 없다.
    let keyed = json!({"command": command, "idempotency_key": "decision-private-key"});
    let first = kernel_request(&mut client, "first", "akra/kernel/command", keyed.clone()).await;
    assert_eq!(first["result"]["processed"], true, "{first}");
    assert!(first["result"]["failure"].is_string(), "{first}");
    assert!(!first.to_string().contains(SECRET));
    let replay = kernel_request(&mut client, "replay", "akra/kernel/command", keyed).await;
    assert_eq!(replay["result"]["operation"]["replayed"], true, "{replay}");
    assert_eq!(
        first["result"]["operation"]["op_id"],
        replay["result"]["operation"]["op_id"]
    );
    assert!(!replay.to_string().contains(SECRET));

    let operation = kernel_request(
        &mut client,
        "operation",
        "akra/kernel/query",
        json!({"query": {"type": "operation", "op_id": first["result"]["operation"]["op_id"]}}),
    )
    .await;
    assert!(!operation.to_string().contains(SECRET));
    assert_eq!(operation["result"]["state"], "failed", "{operation}");
    kernel_request(
        &mut client,
        "recording-marker",
        "akra/gateway/info",
        json!({}),
    )
    .await;
    let recorded: String = std::fs::read_dir(&recordings)
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect();
    assert!(
        recorded.contains("akra/gateway/info"),
        "녹화가 켜져 있음을 확인한다"
    );
    assert!(
        !recorded.contains(SECRET),
        "원문 입력과 decoder 오류는 녹화에 없다"
    );
    kernel_request(&mut client, "detach", "akra/kernel/detach", json!({})).await;
    client.close(None).await.unwrap();
    drop(client);
    tokio::time::timeout(TEST_TIMEOUT, session)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    kernels.shutdown();
}
