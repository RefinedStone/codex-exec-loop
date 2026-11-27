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
    let (local, other) = tokio::join!(
        kernel_request(
            &mut first,
            "local",
            "akra/kernel/extensions/attach",
            json!({"scope":"local_user","client_name":"local"}),
        ),
        kernel_request(
            &mut second,
            "local",
            "akra/kernel/extensions/attach",
            json!({"scope":"local_user","client_name":"other"}),
        )
    );
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
    assert_eq!(repo["result"][0]["id"], "akra.example", "{repo}");
    assert_eq!(repo["result"][1]["id"], "akra.tech-blog", "{repo}");
    assert_eq!(
        repo["result"][0]["enabled"], false,
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

#[tokio::test(flavor = "multi_thread")]
async fn machine_cli_and_desktop_receive_the_same_extension_rejections() {
    use crate::domain::kernel_extension::ExtensionCallRejection;
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut desktop = connect_kernel_client(kernels.clone()).await;
    let mut cli = connect_kernel_client(kernels.clone()).await;
    let attached = kernel_request(
        &mut desktop,
        "attach",
        "akra/kernel/extensions/attach",
        json!({"scope":"local_user","client_name":"rejection-comparison"}),
    )
    .await;
    let host = attached["result"]["kernel"].clone();
    for (name, extension, version, input) in [
        ("disabled", "akra.example", 1, json!({"text":"hello"})),
        ("missing", "akra.missing", 1, json!({"text":"hello"})),
        ("version", "akra.example", 2, json!({"text":"hello"})),
        ("invalid", "akra.example", 1, json!({"text":9})),
    ] {
        let call = json!({"extension":extension,"version":version,"action":"echo","input":input});
        let cli_error = extension_gateway_client::request_over_socket(
            &mut cli,
            "local_user",
            "",
            Some(call.clone()),
        )
        .await
        .unwrap_err();
        let mut params = call;
        params["kernel"] = host.clone();
        let desktop_reply =
            kernel_request(&mut desktop, name, "akra/kernel/extensions/call", params).await;
        let cli_error = cli_error
            .downcast_ref::<ExtensionCallRejection>()
            .expect("typed Gateway rejection");
        assert_eq!(
            serde_json::to_value(cli_error).unwrap(),
            desktop_reply["error"],
            "{name}"
        );
    }
    let _ = desktop.close(None).await;
    let _ = cli.close(None).await;
    kernels.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn local_job_survives_repository_switch_detach_and_client_reconnect() {
    let a = KernelTestFolder::new(&format!("extension-a-{:032x}", rand::random::<u128>()));
    let b = KernelTestFolder::new(&format!("extension-b-{:032x}", rand::random::<u128>()));
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut client = connect_kernel_client(kernels.clone()).await;
    let repository_a =
        attach_kernel(&mut client, "a", &a.workspace()).await["result"]["kernel"].clone();
    let local = kernel_request(
        &mut client,
        "local",
        "akra/kernel/extensions/attach",
        json!({"scope":"local_user","client_name":"scope-lifetime"}),
    )
    .await["result"]["kernel"]
        .clone();
    let enabled = kernel_request(
        &mut client,
        "enable",
        "akra/kernel/extensions/call",
        json!({
            "kernel":local,"extension":"akra.example","version":1,"action":"configure",
            "input":{"enabled":true,"expected_revision":0}
        }),
    )
    .await;
    assert_eq!(enabled["result"]["enabled"], true, "{enabled}");
    let job = kernel_request(
        &mut client,
        "start",
        "akra/kernel/extensions/call",
        json!({
            "kernel":local,"extension":"akra.example","version":1,"action":"count",
            "input":{"steps":100,"delay_ms":25},"idempotency_key":"scope-switch"
        }),
    )
    .await;
    let id = job["result"]["id"]
        .as_str()
        .expect("durable job ID")
        .to_owned();
    let repository_b =
        attach_kernel(&mut client, "b", &b.workspace()).await["result"]["kernel"].clone();
    assert_ne!(repository_a, repository_b);
    let observed = kernel_request(&mut client,"job","akra/kernel/extensions/call",json!({
        "kernel":local,"extension":"akra.example","version":1,"action":"job","input":{"job_id":id}
    })).await;
    assert_eq!(observed["result"]["id"], id, "{observed}");
    let detached = kernel_request(
        &mut client,
        "detach",
        "akra/kernel/extensions/detach",
        json!({"kernel":local}),
    )
    .await;
    assert!(detached["result"].is_object(), "{detached}");
    let _ = client.close(None).await;
    let mut reconnected = connect_kernel_client(kernels.clone()).await;
    let attached = kernel_request(
        &mut reconnected,
        "local",
        "akra/kernel/extensions/attach",
        json!({"scope":"local_user","client_name":"reconnected"}),
    )
    .await;
    assert_eq!(attached["result"]["kernel"], local);
    let observed = kernel_request(&mut reconnected,"job","akra/kernel/extensions/call",json!({
        "kernel":local,"extension":"akra.example","version":1,"action":"job","input":{"job_id":id}
    })).await;
    assert_eq!(observed["result"]["id"], id, "{observed}");
    assert!(matches!(
        observed["result"]["state"].as_str(),
        Some("queued" | "running" | "completed")
    ));
    let _ = reconnected.close(None).await;
    kernels.shutdown();
}
