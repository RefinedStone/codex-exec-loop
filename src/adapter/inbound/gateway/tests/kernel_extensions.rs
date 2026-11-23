use super::*;
use crate::adapter::outbound::extension_gateway_client;

#[tokio::test(flavor = "multi_thread")]
async fn local_extension_attachment_does_not_replace_repository_and_two_clients_share_state() {
    let folder = KernelTestFolder::new(&format!("extension-scope-{:032x}", rand::random::<u128>()));
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut first = connect_kernel_client(kernels.clone()).await;
    let mut second = connect_kernel_client(kernels.clone()).await;
    let repository =
        attach_kernel(&mut first, "repo", &folder.workspace()).await["result"]["kernel"].clone();
    let local = kernel_request(
        &mut first,
        "local",
        "akra/kernel/extensions/attach",
        json!({"scope":"local_user","client_name":"local"}),
    )
    .await;
    let other = kernel_request(
        &mut second,
        "local",
        "akra/kernel/extensions/attach",
        json!({"scope":"local_user","client_name":"other"}),
    )
    .await;
    assert_eq!(
        local["result"]["kernel"], other["result"]["kernel"],
        "{local} {other}"
    );
    let host = local["result"]["kernel"].clone();
    let enable=kernel_request(&mut first,"enable","akra/kernel/extensions/call",json!({
        "kernel":host,"extension":"akra.example","version":1,"action":"configure","input":{"enabled":true,"expected_revision":0}
    })).await;
    assert_eq!(enable["result"]["enabled"], true, "{enable}");
    let notice = next_json_matching(&mut second, |v| {
        v["method"] == "akra/kernel/extensions/changed"
    })
    .await;
    assert_eq!(notice["params"]["kernel"], host);
    let observed = kernel_request(
        &mut second,
        "list",
        "akra/kernel/extensions/list",
        json!({"kernel":host}),
    )
    .await;
    assert_eq!(observed["result"][0]["enabled"], true);
    let repo = kernel_request(
        &mut first,
        "repo-list",
        "akra/kernel/extensions/list",
        json!({"kernel":repository}),
    )
    .await;
    assert_eq!(repo["result"][0]["id"], "akra.tech-blog", "{repo}");
    assert_eq!(
        repo["result"][1]["enabled"], false,
        "repository activation stays independent"
    );
    let detached = kernel_request(
        &mut first,
        "detach-local",
        "akra/kernel/extensions/detach",
        json!({"kernel":host}),
    )
    .await;
    assert!(detached["result"].is_object(), "{detached}");
    let repo = kernel_request(
        &mut first,
        "repo-again",
        "akra/kernel/extensions/list",
        json!({"kernel":repository}),
    )
    .await;
    assert!(repo["result"].is_array(), "{repo}");
    let unavailable = kernel_request(
        &mut first,
        "stale-local",
        "akra/kernel/extensions/list",
        json!({"kernel":host}),
    )
    .await;
    assert!(unavailable["error"].is_object(), "{unavailable}");
    let _ = first.close(None).await;
    let _ = second.close(None).await;
    kernels.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn machine_cli_transport_uses_the_catalog_schema_and_same_host() {
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut client = connect_kernel_client(kernels.clone()).await;
    let catalog =
        extension_gateway_client::request_over_socket(&mut client, "local_user", "", None)
            .await
            .unwrap();
    assert_eq!(catalog[0]["id"], "akra.example");
    let enable=extension_gateway_client::request_over_socket(&mut client,"local_user","",Some(json!({
        "extension":"akra.example","version":1,"action":"configure","input":{"enabled":true,"expected_revision":0}
    }))).await.unwrap();
    assert_eq!(enable["enabled"], true);
    let reply = extension_gateway_client::request_over_socket(
        &mut client,
        "local_user",
        "",
        Some(json!({
            "extension":"akra.example","version":1,"action":"echo","input":{"text":"hello"}
        })),
    )
    .await
    .unwrap();
    assert_eq!(reply["text"], "hello");
    let invalid = extension_gateway_client::request_over_socket(
        &mut client,
        "local_user",
        "",
        Some(json!({
            "extension":"akra.example","version":1,"action":"echo","input":{"text":9}
        })),
    )
    .await;
    assert!(invalid.is_err());
    let _ = client.close(None).await;
    kernels.shutdown();
}
