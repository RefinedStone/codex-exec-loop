//! 실제 WebSocket 녹화 경계에서 steer 원문과 decoder 오류를 보호한다.
use super::*;
use crate::composition::repository_kernel::RepositoryKernelRegistry;

#[tokio::test(flavor = "multi_thread")]
async fn issue_intervention_guidance_and_decoder_errors_stay_out_of_gateway_recordings() {
    const SECRET: &str = "steer-private-guidance-canary-2549";
    let folder = KernelTestFolder::new("intervention-privacy");
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
    let issue = crate::domain::issue_work::IssueReference {
        provider: crate::domain::issue_work::IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_gateway_intervention".into(),
        issue_id: "I_gateway_intervention".into(),
        project_scope: crate::domain::issue_work::IssueProjectScope {
            workspace_dir: folder.workspace(),
        },
    };
    let command = json!({
        "type": "steer_issue",
        "issue": crate::domain::kernel_event::IssueEventReference::from(&issue),
        "command_id": "gateway-intervention",
        "message": SECRET,
    });
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
        "invalid issue steer parameters"
    );
    assert!(!rejected.to_string().contains(SECRET));

    // 빈 지시는 서비스를 부르기 전에 거절하고, 입력 원문은 응답과 녹화에 넣지 않는다.
    let mut invalid = command;
    invalid["message"] = json!(" ".repeat(65537));
    let rejected = kernel_request(
        &mut client,
        "invalid",
        "akra/kernel/command",
        json!({"command": invalid}),
    )
    .await;
    assert_eq!(rejected["error"]["code"], -32602);
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
        "the recorder is enabled"
    );
    assert!(!recorded.contains(SECRET));
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
