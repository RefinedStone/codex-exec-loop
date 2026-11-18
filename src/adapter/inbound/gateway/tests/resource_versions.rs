use super::*;
use crate::application::port::outbound::external_resource_version_port::ExternalResourceVersionPort;
#[cfg(test)]
use crate::application::service::resource_version::ResourceVersionService;
use crate::domain::work_context::ExternalResourceRef;

async fn provider_client(
    service: Arc<ResourceVersionService>,
    remote: bool,
) -> WebSocketStream<DuplexStream> {
    let (client, gateway) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
    let local = EchoUpstream::new(true);
    let cloud = EchoUpstream::new(true);
    tokio::spawn(run_session(
        gateway,
        GatewayHosts::new(local, vec![("cloud".into(), cloud)]),
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default()),
        super::super::session::GatewayServices {
            resource_versions: Some(service),
            ..Default::default()
        },
        None,
    ));
    let path = if remote {
        "ws://localhost/host/cloud"
    } else {
        "ws://localhost/"
    };
    tokio_tungstenite::client_async_with_config(path, client, Some(websocket_config()))
        .await
        .unwrap()
        .0
}

fn resource() -> ExternalResourceRef {
    ExternalResourceRef {
        kind: "jira".into(),
        key: "acme.example/APP-1".into(),
        url: "https://acme.example/browse/APP-1".into(),
        title_hint: None,
    }
}

#[tokio::test]
async fn reverse_answers_are_consumed_while_normal_messages_still_relay() {
    let service = Arc::new(ResourceVersionService::default());
    let mut client = provider_client(service.clone(), false).await;
    let registered = kernel_request(
        &mut client,
        "register",
        "akra/resourceVersion/register",
        json!({"kind":"jira","siteUrl":"https://acme.example"}),
    )
    .await;
    assert!(registered.get("result").is_some());
    let worker = service.clone();
    let lookup = tokio::task::spawn_blocking(move || worker.current_version(&resource()));
    let request: Value = serde_json::from_str(&next_text(&mut client).await).unwrap();
    assert_eq!(request["method"], "akra/resourceVersion/current");
    client
        .send(Message::text(
            json!({"jsonrpc":"2.0","id":request["id"],
        "result":{"version":"2026-11-17T00:00:00Z"}})
            .to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(TEST_TIMEOUT, lookup)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        "2026-11-17T00:00:00Z"
    );
    // A duplicate/late provider answer must not appear in upstream's echo.
    client
        .send(Message::text(
            json!({"id":request["id"],"result":{"version":"ignored"}}).to_string(),
        ))
        .await
        .unwrap();
    client.send(Message::text("normal")).await.unwrap();
    assert_eq!(next_text(&mut client).await, "echo:normal");
}

#[tokio::test]
async fn closing_a_socket_cancels_the_pending_provider_call() {
    let service = Arc::new(ResourceVersionService::default());
    let mut client = provider_client(service.clone(), false).await;
    kernel_request(
        &mut client,
        "register",
        "akra/resourceVersion/register",
        json!({"kind":"jira","siteUrl":"https://acme.example"}),
    )
    .await;
    let worker = service.clone();
    let lookup = tokio::task::spawn_blocking(move || worker.current_version(&resource()));
    next_text(&mut client).await;
    client.close(None).await.unwrap();
    assert!(
        tokio::time::timeout(TEST_TIMEOUT, lookup)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(!service.available(&resource()));
}

#[tokio::test]
async fn remote_connections_cannot_register_or_forward_provider_responses() {
    let service = Arc::new(ResourceVersionService::default());
    let mut client = provider_client(service.clone(), true).await;
    let registered = kernel_request(
        &mut client,
        "register",
        "akra/resourceVersion/register",
        json!({"kind":"jira","siteUrl":"https://acme.example"}),
    )
    .await;
    assert_eq!(registered["error"]["code"], -32601);
    assert!(!service.available(&resource()));
    client
        .send(Message::text(
            r#"{"id":"akra/resourceVersion/request/1","result":{"version":"ignored"}}"#,
        ))
        .await
        .unwrap();
    client.send(Message::text("normal")).await.unwrap();
    assert_eq!(next_text(&mut client).await, "echo:normal");
}
