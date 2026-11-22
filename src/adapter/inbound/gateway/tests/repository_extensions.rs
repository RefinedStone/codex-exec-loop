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
    let enable = json!({"kernel":kernel,"extension":"akra.tech-blog","version":1,"action":"configure","input":{"enabled":true}});
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
