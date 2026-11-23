use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn blog_changes_reach_another_attached_desktop_and_reject_stale_scope() {
    let folder = KernelTestFolder::new(&format!("blog-extension-{:032x}", rand::random::<u128>()));
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut editor = connect_kernel_client(kernels.clone()).await;
    let mut observer = connect_kernel_client(kernels).await;
    let attach = attach_kernel(&mut editor, "editor", &folder.workspace()).await;
    attach_kernel(&mut observer, "observer", &folder.workspace()).await;
    let kernel = &attach["result"]["kernel"];
    let list = "akra/kernel/extensions/list";
    let call = "akra/kernel/extensions/call";
    let catalog = kernel_request(&mut observer, "before", list, json!({"kernel":kernel})).await;
    assert_eq!(catalog["result"][0]["id"], "akra.tech-blog", "{catalog}");
    assert_eq!(catalog["result"][0]["enabled"], false);
    let enable = json!({"kernel":kernel,"extension":"akra.tech-blog","version":2,"action":"configure","input":{"enabled":true}});
    let mut stale = enable.clone();
    stale["kernel"] = json!("another-repository");
    let denied = kernel_request(&mut editor, "stale", call, stale).await;
    assert!(denied["error"].is_object(), "{denied}");
    let changed = kernel_request(&mut editor, "enable", call, enable).await;
    assert_eq!(changed["result"]["enabled"], true, "{changed}");
    let notice = next_json_matching(&mut observer, |value| {
        value["method"] == "akra/kernel/extensions/changed"
    })
    .await;
    assert_eq!(
        notice["params"],
        json!({"kernel": kernel, "extension": "akra.tech-blog"})
    );
    let after = kernel_request(&mut observer, "after", list, json!({"kernel":kernel})).await;
    assert_eq!(after["result"][0]["enabled"], true, "{after}");
}

#[tokio::test(flavor = "multi_thread")]
async fn machine_cli_legacy_blog_creates_once_and_preserves_worker_editing_contracts() {
    use crate::adapter::outbound::extension_gateway_client;
    let folder = KernelTestFolder::new(&format!("blog-cli-{:032x}", rand::random::<u128>()));
    std::fs::write(
        folder.path.join("README.md"),
        "# Fixture\nObserved source.\n",
    )
    .unwrap();
    for args in [
        vec!["init"],
        vec!["add", "README.md"],
        vec![
            "-c",
            "user.name=Akra Test",
            "-c",
            "user.email=akra@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    ] {
        let output = crate::test_utils::git_command()
            .current_dir(&folder.path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let kernels =
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default());
    let mut client = connect_kernel_client(kernels.clone()).await;
    let workspace = folder.workspace();
    let catalog =
        extension_gateway_client::request_over_socket(&mut client, "repository", &workspace, None)
            .await
            .unwrap();
    let blog = catalog
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "akra.tech-blog")
        .unwrap();
    assert!(
        blog["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|operation| operation["name"] == "create")
    );
    for (action, input) in [
        ("configure", json!({"enabled":true})),
        (
            "create",
            json!({
                "topic":"Real worker compatibility","style":"TECHNICAL_DEEP_DIVE",
                "mode":"TECH_EXPLORATION","paths":["README.md"]
            }),
        ),
    ] {
        let result = extension_gateway_client::request_over_socket(
            &mut client,
            "repository",
            &workspace,
            Some(json!({"extension":"akra.tech-blog","version":1,"action":action,"input":input})),
        )
        .await
        .unwrap();
        if action == "create" {
            assert_eq!(result["revision"], 1);
            assert_eq!(result["sources"][0]["path"], "README.md");
        }
    }
    let articles = extension_gateway_client::request_over_socket(
        &mut client,
        "repository",
        &workspace,
        Some(json!({"extension":"akra.tech-blog","version":1,"action":"list","input":{}})),
    )
    .await
    .unwrap();
    assert_eq!(
        articles.as_array().unwrap().len(),
        1,
        "successful creation must not be retried"
    );
    let id = articles[0]["id"].clone();
    for (revision, draft) in [(1, Some("Updated draft")), (2, None)] {
        let mut input = json!({"id":id,"revision":revision});
        if let Some(draft) = draft {
            input["draft"] = json!(draft);
        }
        let saved = extension_gateway_client::request_over_socket(
            &mut client,
            "repository",
            &workspace,
            Some(json!({"extension":"akra.tech-blog","version":1,"action":"save","input":input})),
        )
        .await
        .unwrap();
        assert_eq!(saved["revision"], revision + 1);
        assert_eq!(saved["draft"], draft.unwrap_or(""));
    }
    let rejected = extension_gateway_client::request_over_socket(
        &mut client,
        "repository",
        &workspace,
        Some(
            json!({"extension":"akra.tech-blog","version":1,"action":"save",
            "input":{"id":id,"revision":3,"draft":42}}),
        ),
    )
    .await
    .unwrap_err();
    assert!(
        rejected
            .downcast_ref::<crate::domain::kernel_extension::ExtensionCallRejection>()
            .is_some()
    );
    let current = extension_gateway_client::request_over_socket(
        &mut client,
        "repository",
        &workspace,
        Some(json!({"extension":"akra.tech-blog","version":1,"action":"open","input":{"id":id}})),
    )
    .await
    .unwrap();
    assert_eq!(current["revision"], 3);
    let _ = client.close(None).await;
    kernels.shutdown();
}
