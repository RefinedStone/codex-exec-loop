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
    let blog = |catalog: &Value| {
        catalog["result"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == "akra.tech-blog")
            .cloned()
            .unwrap_or_else(|| panic!("{catalog}"))
    };
    let catalog = kernel_request(&mut observer, "before", list, json!({"kernel":kernel})).await;
    assert_eq!(blog(&catalog)["enabled"], false);
    let enable = json!({"kernel":kernel,"extension":"akra.tech-blog","version":3,"action":"configure",
        "input":{"enabled":true,"expected_revision":blog(&catalog)["revision"]}});
    let mut stale = enable.clone();
    stale["kernel"] = json!("another-repository");
    let denied = kernel_request(&mut editor, "stale", call, stale).await;
    assert!(denied["error"].is_object(), "{denied}");
    let changed = kernel_request(&mut editor, "enable", call, enable).await;
    assert!(changed["error"].is_null(), "{changed}");
    let notice = next_json_matching(&mut observer, |value| {
        value["method"] == "akra/kernel/extensions/changed"
            && value["params"]["extension"] == "akra.tech-blog"
    })
    .await;
    assert_eq!(notice["params"]["kernel"], *kernel);
    let after = kernel_request(&mut observer, "after", list, json!({"kernel":kernel})).await;
    assert_eq!(blog(&after)["enabled"], true, "{after}");
}

#[tokio::test(flavor = "multi_thread")]
async fn machine_cli_blog_v3_preserves_saved_articles_without_model_calls() {
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
    assert_eq!(blog["version"], 3);
    assert!(
        blog["permissions"]
            .as_array()
            .unwrap()
            .contains(&json!("workspace.write"))
    );
    let actions = blog["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|operation| operation["name"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        actions,
        std::collections::BTreeSet::from([
            "configure",
            "jobs",
            "job",
            "cancel",
            "targets",
            "list",
            "open",
            "seen",
            "generate",
            "stop",
            "resume",
            "revise",
            "review",
            "save",
            "restore",
            "figure",
            "export",
            "delete",
        ])
    );
    let configured = blog_cli_request(
        &mut client,
        &workspace,
        "configure",
        json!({"enabled":true,"expected_revision":blog["revision"]}),
    )
    .await
    .unwrap();
    assert_eq!(configured["enabled"], true, "{configured}");
    let targets = blog_cli_request(&mut client, &workspace, "targets", json!({"limit":1}))
        .await
        .unwrap();
    assert_eq!(targets["commits"].as_array().unwrap().len(), 1);
    assert_eq!(targets["limit"], 30);
    let commit = targets["commits"][0]["sha"].clone();

    // Seed saved data only. Successful model and figure entrances use fake sockets elsewhere;
    // these real Gateway calls must never start an article's model stages.
    let legacy = json!({
        "id":"legacy", "topic":"Saved version one article", "draft":"Existing draft",
        "revision":3, "job":null, "review":{"state":"reviewed","findings":[]},
        "figures":[], "messages":[]
    });
    let current = json!({
        "id":"current", "version":2, "title":"Stopped article", "topic":"Saved fixture",
        "commits":[commit], "state":"stopped", "stage":"write", "attempt":0, "calls":0,
        "evidence":null, "style":null, "draft":"Saved draft", "claims":[], "figures":[],
        "review":null, "stopped":{"reason":"cancelled","code":"cancelled","message":"fixture"},
        "messages":[], "revision":1, "created":1, "updated":1, "job":null
    });
    let finished_id = "0123456789abcdef0123456789abcdef";
    let mut finished = current.clone();
    finished["id"] = json!(finished_id);
    finished["title"] = json!("Saved complete article");
    finished["state"] = json!("reviewed");
    finished["unread"] = json!(true);
    finished["stage"] = Value::Null;
    finished["stopped"] = Value::Null;
    finished["figures"] = json!([
        {"id":"fig1","path":"fig1.png","alt":"Stored figure","caption":"Stored figure"}
    ]);
    {
        let runtime = crate::composition::production::delegation_runtime(&workspace).unwrap();
        let db = rusqlite::Connection::open(runtime.join("tech-blog.sqlite3")).unwrap();
        for article in [legacy, current, finished] {
            db.execute(
                "INSERT INTO article VALUES (?1,?2)",
                rusqlite::params![article["id"].as_str().unwrap(), article.to_string()],
            )
            .unwrap();
        }
        db.execute(
            "INSERT INTO image VALUES (?1,?2,?3)",
            rusqlite::params![finished_id, "fig1", &b"stored image fixture"[..]],
        )
        .unwrap();
    }
    let articles = blog_cli_request(&mut client, &workspace, "list", json!({}))
        .await
        .unwrap();
    assert_eq!(articles.as_array().unwrap().len(), 3);
    for article in articles.as_array().unwrap() {
        assert!(article.get("attempt").is_none());
        assert_eq!(article["unread"], article["id"] == finished_id);
        assert_eq!(article["failed"], article["id"] == "current");
        if article["id"] == "legacy" {
            assert!(article["task"].is_null());
        } else {
            assert_eq!(article["task"], "generate");
        }
    }
    let legacy = blog_cli_request(&mut client, &workspace, "open", json!({"id":"legacy"}))
        .await
        .unwrap();
    assert_eq!(legacy["version"], 1);
    assert_eq!(legacy["state"], "reviewed");
    assert!(legacy["stage"].is_null());
    let output_schema = &blog["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["name"] == "open")
        .unwrap()["output_schema"];
    assert!(
        jsonschema::validator_for(output_schema)
            .unwrap()
            .is_valid(&legacy)
    );

    let finished = blog_cli_request(&mut client, &workspace, "open", json!({"id":finished_id}))
        .await
        .unwrap();
    assert_eq!(finished["edition"], 1);
    assert_eq!(finished["task"], "generate");
    assert!(
        finished["previous"].is_null()
            && finished["exported"].is_null()
            && finished["slug"].is_null()
    );
    assert_eq!(finished["unread"], true);
    let seen = blog_cli_request(&mut client, &workspace, "seen", json!({"id":finished_id}))
        .await
        .unwrap();
    let mut expected_seen = finished.clone();
    expected_seen["unread"] = json!(false);
    assert_eq!(
        seen, expected_seen,
        "seen must preserve revision, updated, edition and article content"
    );
    assert_eq!(
        blog_cli_request(&mut client, &workspace, "seen", json!({"id":finished_id}))
            .await
            .unwrap(),
        seen,
        "repeated seen does not edit the article"
    );
    assert_eq!(
        blog_cli_request(&mut client, &workspace, "seen", json!({"id":"legacy"}))
            .await
            .unwrap(),
        legacy,
        "saved v1 articles remain compatible"
    );
    let after_seen = blog_cli_request(&mut client, &workspace, "list", json!({}))
        .await
        .unwrap();
    let summary = after_seen
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == finished_id)
        .unwrap();
    assert_eq!(summary["unread"], false);
    assert_eq!(summary["failed"], false);
    let saved = blog_cli_request(
        &mut client,
        &workspace,
        "save",
        json!({"id":finished_id,"revision":1,"draft":"Edited complete draft"}),
    )
    .await
    .unwrap();
    assert_eq!(saved["edition"], 2);
    assert_eq!(saved["previous"]["draft"], "Saved draft");
    assert_eq!(saved["state"], "edited");
    let restored = blog_cli_request(
        &mut client,
        &workspace,
        "restore",
        json!({"id":finished_id,"revision":2}),
    )
    .await
    .unwrap();
    assert_eq!(restored["revision"], 3);
    assert_eq!(restored["edition"], 3);
    assert_eq!(restored["draft"], "Saved draft");
    assert_eq!(restored["previous"]["draft"], "Edited complete draft");
    assert_eq!(restored["state"], "reviewed");
    let exported = blog_cli_request(
        &mut client,
        &workspace,
        "export",
        json!({"id":finished_id,"revision":3}),
    )
    .await
    .unwrap();
    assert_eq!(exported["revision"], 4);
    assert_eq!(exported["exported"]["edition"], 3);
    assert_eq!(
        exported["exported"]["files"],
        json!(["post.md", "fig1.png"])
    );
    let post = folder
        .path
        .join(exported["exported"]["path"].as_str().unwrap());
    let contents = std::fs::read_to_string(&post).unwrap();
    assert!(contents.contains(&format!("akra_article: {finished_id}")));
    assert!(contents.ends_with("Saved draft\n"));
    assert_eq!(
        std::fs::read(post.parent().unwrap().join("fig1.png")).unwrap(),
        b"stored image fixture"
    );
    let exported_again = blog_cli_request(
        &mut client,
        &workspace,
        "export",
        json!({"id":finished_id,"revision":4}),
    )
    .await
    .unwrap();
    assert_eq!(exported_again["revision"], 5);
    assert_eq!(
        exported_again["exported"]["path"],
        exported["exported"]["path"]
    );

    // Seed a later saved figure without invoking figure/model work. Export ownership must not
    // claim a pre-existing image just because this article already owns post.md in the folder.
    let mut with_new_figure = exported_again.clone();
    with_new_figure["revision"] = json!(6);
    with_new_figure["edition"] = json!(4);
    with_new_figure["figures"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id":"fig2","path":"fig2.png","alt":"Later figure","caption":"Later figure"
        }));
    {
        let runtime = crate::composition::production::delegation_runtime(&workspace).unwrap();
        let db = rusqlite::Connection::open(runtime.join("tech-blog.sqlite3")).unwrap();
        db.execute(
            "UPDATE article SET value=?1 WHERE id=?2",
            rusqlite::params![with_new_figure.to_string(), finished_id],
        )
        .unwrap();
        db.execute(
            "INSERT INTO image VALUES (?1,?2,?3)",
            rusqlite::params![finished_id, "fig2", &b"later stored image"[..]],
        )
        .unwrap();
    }
    let foreign_image = post.parent().unwrap().join("fig2.png");
    std::fs::write(&foreign_image, b"another author's image").unwrap();
    let rejected_export = blog_cli_request(
        &mut client,
        &workspace,
        "export",
        json!({"id":finished_id,"revision":6}),
    )
    .await
    .unwrap_err();
    assert!(
        rejected_export
            .downcast_ref::<crate::domain::kernel_extension::ExtensionCallRejection>()
            .is_some()
    );
    assert_eq!(
        std::fs::read(&foreign_image).unwrap(),
        b"another author's image"
    );
    assert_eq!(std::fs::read_to_string(&post).unwrap(), contents);
    let unchanged = blog_cli_request(&mut client, &workspace, "open", json!({"id":finished_id}))
        .await
        .unwrap();
    assert_eq!(unchanged, with_new_figure);
    assert_eq!(unchanged["calls"], 0);
    assert!(unchanged["job"].is_null());

    for (revision, draft) in [(1, Some("Updated draft")), (2, None)] {
        let mut input = json!({"id":"current","revision":revision});
        if let Some(draft) = draft {
            input["draft"] = json!(draft);
        }
        let saved = blog_cli_request(&mut client, &workspace, "save", input)
            .await
            .unwrap();
        assert_eq!(saved["revision"], revision + 1);
        assert_eq!(saved["draft"], draft.unwrap_or(""));
        assert_eq!(saved["state"], "stopped");
        assert_eq!(saved["calls"], 0);
    }
    let rejected = blog_cli_request(
        &mut client,
        &workspace,
        "save",
        json!({"id":"current","revision":3,"draft":42}),
    )
    .await
    .unwrap_err();
    assert!(
        rejected
            .downcast_ref::<crate::domain::kernel_extension::ExtensionCallRejection>()
            .is_some()
    );
    let current = blog_cli_request(&mut client, &workspace, "stop", json!({"id":"current"}))
        .await
        .unwrap();
    assert_eq!(current["revision"], 3);
    assert_eq!(current["calls"], 0);
    assert!(current["job"].is_null());
    assert!(
        blog_cli_request(&mut client, &workspace, "generate", json!({"commits":[]}))
            .await
            .is_err()
    );
    assert!(
        extension_gateway_client::request_over_socket(
            &mut client,
            "repository",
            &workspace,
            Some(json!({"extension":"akra.tech-blog","version":2,"action":"list","input":{}})),
        )
        .await
        .is_err()
    );
    for id in ["legacy", "current", finished_id] {
        let deleted = blog_cli_request(&mut client, &workspace, "delete", json!({"id":id}))
            .await
            .unwrap();
        assert_eq!(deleted, json!({"id":id}));
    }
    assert_eq!(
        blog_cli_request(&mut client, &workspace, "list", json!({}))
            .await
            .unwrap(),
        json!([])
    );
    let _ = client.close(None).await;
    kernels.shutdown();
}

async fn blog_cli_request(
    client: &mut WebSocketStream<DuplexStream>,
    workspace: &str,
    action: &str,
    input: Value,
) -> anyhow::Result<Value> {
    crate::adapter::outbound::extension_gateway_client::request_over_socket(
        client,
        "repository",
        workspace,
        Some(json!({"extension":"akra.tech-blog","version":3,"action":action,"input":input})),
    )
    .await
}
