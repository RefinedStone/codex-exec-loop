use super::*;
use tokio_tungstenite::tungstenite::protocol::Role;

async fn call_with_catalog_response(catalog: Value, call: Value, response: Value) -> Result<Value> {
    let (client_io, server_io) = tokio::io::duplex(16384);
    let mut client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
    let mut server = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
    let expected_call = call.clone();
    let task = tokio::spawn(async move {
        let mut calls = 0;
        for step in 0..3 {
            let message = server.next().await.unwrap().unwrap();
            let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            let mut answer = json!({"jsonrpc":"2.0","id":request["id"]});
            match step {
                0 => answer["result"] = json!({"kernel":"test-host"}),
                1 => answer["result"] = catalog.clone(),
                _ => {
                    calls += 1;
                    assert_eq!(request["method"], "akra/kernel/extensions/call");
                    assert_eq!(request["params"]["input"], expected_call["input"]);
                    assert_eq!(request["params"]["action"], expected_call["action"]);
                    assert_eq!(request["params"]["version"], expected_call["version"]);
                    assert_eq!(request["params"]["extension"], expected_call["extension"]);
                    for (key, value) in response.as_object().unwrap() {
                        answer[key] = value.clone();
                    }
                }
            }
            server
                .send(Message::text(answer.to_string()))
                .await
                .unwrap();
        }
        assert_eq!(calls, 1);
    });
    let answer = request_over_socket(&mut client, "repository", "fixture", Some(call)).await;
    task.await.unwrap();
    answer
}

async fn call_with_server_response(response: Value) -> Result<Value> {
    call_with_catalog_response(
        json!([{
            "id":"akra.example","version":1,"operations":[{
                "name":"echo","input_schema":{
                    "type":"object","required":["text"],"properties":{"text":{"type":"string"}},
                    "additionalProperties":false
                }
            }]
        }]),
        json!({"extension":"akra.example","version":1,"action":"echo","input":{"text":1}}),
        response,
    )
    .await
}

#[tokio::test]
async fn catalog_preflight_rejection_preserves_server_reason_and_data() {
    let expected =
        json!({"code":-32003,"message":"input rejected by host","data":{"reason":"invalid_input"}});
    let error = call_with_server_response(json!({"error":expected}))
        .await
        .unwrap_err();
    let rejection = error.downcast_ref::<ExtensionCallRejection>().unwrap();
    assert_eq!(serde_json::to_value(rejection).unwrap(), expected);
}

#[tokio::test]
async fn a_server_accepting_input_rejected_by_its_catalog_is_a_contract_error() {
    let error = call_with_server_response(json!({"result":{"text":1}}))
        .await
        .unwrap_err();
    let rejection = error.downcast_ref::<ExtensionCallRejection>().unwrap();
    assert_eq!(rejection.code, -32603);
    assert_eq!(
        rejection.data.as_ref().unwrap()["reason"],
        "extension_catalog_mismatch"
    );
    assert_eq!(rejection.data.as_ref().unwrap()["source"], "client");
}

fn blog_catalog() -> Value {
    json!([{
        "id":"akra.tech-blog","version":2,
        "operations":crate::domain::repository_extension::tech_blog::operations()
    }])
}

fn validate_blog_call(action: &str, input: Value) -> Result<()> {
    validate_catalog_call(
        &blog_catalog(),
        &json!({"extension":"akra.tech-blog","version":2,"action":action,"input":input}),
    )
}

#[tokio::test]
async fn blog_v2_job_success_is_returned_once_without_catalog_mismatch() {
    // Fake responses cover every model job entrance without scheduling a worker or model.
    for (action, input) in [
        ("generate", json!({"commits":["a".repeat(40)]})),
        ("resume", json!({"id":"article","revision":2})),
        (
            "revise",
            json!({"id":"article","revision":2,"instruction":"Clarify the evidence"}),
        ),
        ("review", json!({"id":"article","revision":2})),
    ] {
        let result =
            json!({"id":"article","version":2,"revision":3,"job":{"id":"already-started"}});
        let answer = call_with_catalog_response(
            blog_catalog(),
            json!({"extension":"akra.tech-blog","version":2,"action":action,"input":input}),
            json!({"result":result}),
        )
        .await
        .unwrap();
        assert_eq!(answer, result);
    }
}

#[tokio::test]
async fn blog_v2_figure_success_preserves_edition_and_export_metadata() {
    // Figure registration is a fake response too: do not invoke model or image workers here.
    let result = json!({
        "id":"article","version":2,"revision":4,"task":"generate","state":"edited",
        "job":null,"edition":2,"slug":"saved-article","draft":"# Edited\n![figure](fig1.png)\n",
        "previous":{"title":"Saved","style":"PROBLEM_SOLVING","slug":"saved-article",
            "draft":"# Saved","claims":[],"review":null,"state":"edited"},
        "exported":{"path":"blog/2026-11-23-saved-article/post.md","edition":1,"at":1,
            "files":["post.md"]},
        "figures":[{"id":"fig1","path":"fig1.png","alt":"figure","caption":"figure"}]
    });
    let answer = call_with_catalog_response(
        blog_catalog(),
        json!({"extension":"akra.tech-blog","version":2,"action":"figure",
            "input":{"id":"article","revision":3,"png":"encoded-image","caption":"figure"}}),
        json!({"result":result}),
    )
    .await
    .unwrap();
    assert_eq!(answer, result);
    let operation = crate::domain::repository_extension::tech_blog::operations()
        .into_iter()
        .find(|entry| entry.name == "figure")
        .unwrap();
    assert!(
        jsonschema::validator_for(&operation.output_schema)
            .unwrap()
            .is_valid(&answer)
    );
}

#[test]
fn blog_v2_catalog_matches_optional_defaults_and_valid_worker_requests() {
    let commits = (0..30).map(|n| format!("{n:040x}")).collect::<Vec<_>>();
    for (action, input) in [
        ("catalog", json!({})),
        ("configure", json!({"enabled":true})),
        ("targets", json!({})),
        ("targets", json!({"limit":1})),
        ("targets", json!({"limit":500})),
        ("list", json!({})),
        ("open", json!({"id":"a".repeat(64)})),
        ("seen", json!({"id":"article"})),
        // Read acknowledgment deliberately does not require or validate an article revision.
        ("seen", json!({"id":"article","revision":null})),
        ("generate", json!({"commits":["a".repeat(40)]})),
        (
            "generate",
            json!({"commits":commits,"topic":"t".repeat(300)}),
        ),
        ("generate", json!({"commits":["b".repeat(40)],"topic":""})),
        ("cancel", json!({"id":"article"})),
        ("resume", json!({"id":"article","revision":1})),
        ("revise", json!({"id":"article","revision":1})),
        (
            "revise",
            json!({"id":"article","revision":1,"instruction":""}),
        ),
        (
            "revise",
            json!({"id":"article","revision":1,"instruction":"요".repeat(1_000)}),
        ),
        ("review", json!({"id":"article","revision":1})),
        // Review ignores the rewrite-only instruction field, as the Python service does.
        (
            "review",
            json!({"id":"article","revision":1,"instruction":42}),
        ),
        ("restore", json!({"id":"article","revision":1})),
        ("export", json!({"id":"article","revision":1})),
        ("save", json!({"id":"article","revision":1})),
        (
            "save",
            json!({"id":"article","revision":1,"draft":"d".repeat(100_000)}),
        ),
        (
            "figure",
            json!({"id":"article","revision":1,"png":"encoded-image","caption":"c".repeat(300)}),
        ),
        ("delete", json!({"id":"article"})),
        // The Python worker ignores unrelated fields; keep that input compatibility.
        ("targets", json!({"limit":1,"extra":"ignored"})),
    ] {
        validate_blog_call(action, input.clone())
            .unwrap_or_else(|error| panic!("{action}: {input}: {error}"));
    }
}

#[test]
fn blog_v2_catalog_rejects_invalid_worker_fields_and_obsolete_actions() {
    let commits = (0..31).map(|n| format!("{n:040x}")).collect::<Vec<_>>();
    for (action, input) in [
        ("configure", json!({})),
        ("configure", json!({"enabled":1})),
        ("targets", json!({"limit":0})),
        ("targets", json!({"limit":501})),
        ("targets", json!({"limit":true})),
        ("targets", json!({"limit":"50"})),
        ("open", json!({"id":" "})),
        ("open", json!({"id":"a".repeat(65)})),
        ("seen", json!({})),
        ("seen", json!({"id":" "})),
        ("seen", json!({"id":42})),
        ("seen", json!({"id":"a".repeat(65)})),
        ("generate", json!({})),
        ("generate", json!({"commits":[]})),
        ("generate", json!({"commits":"a".repeat(40)})),
        (
            "generate",
            json!({"commits":["a".repeat(40),"a".repeat(40)]}),
        ),
        ("generate", json!({"commits":["A".repeat(40)]})),
        ("generate", json!({"commits":["a".repeat(39)]})),
        (
            "generate",
            json!({"commits":[format!("{}\n", "a".repeat(40))]}),
        ),
        ("generate", json!({"commits":commits})),
        ("generate", json!({"commits":["a".repeat(40)],"topic":null})),
        (
            "generate",
            json!({"commits":["a".repeat(40)],"topic":"t".repeat(301)}),
        ),
        ("cancel", json!({})),
        ("resume", json!({"id":"article"})),
        ("resume", json!({"id":"article","revision":0})),
        ("revise", json!({"id":"article"})),
        (
            "revise",
            json!({"id":"article","revision":1,"instruction":null}),
        ),
        (
            "revise",
            json!({"id":"article","revision":1,"instruction":42}),
        ),
        (
            "revise",
            json!({"id":"article","revision":1,"instruction":"요".repeat(1_001)}),
        ),
        ("review", json!({"id":"article"})),
        ("review", json!({"id":"article","revision":0})),
        ("restore", json!({"id":"article"})),
        ("export", json!({"id":"article"})),
        ("export", json!({"id":"article","revision":"1"})),
        ("save", json!({"id":"article","revision":1,"draft":1})),
        (
            "save",
            json!({"id":"article","revision":1,"draft":"d".repeat(100_001)}),
        ),
        (
            "figure",
            json!({"id":"article","revision":1,"caption":"caption"}),
        ),
        (
            "figure",
            json!({"id":"article","revision":1,"png":"encoded-image","caption":" "}),
        ),
        (
            "figure",
            json!({"id":"article","revision":1,"png":"x".repeat(2_800_001),"caption":"caption"}),
        ),
        ("delete", json!({})),
    ] {
        assert!(validate_blog_call(action, input).is_err(), "{action}");
    }
    for action in ["create", "accept", "write", "rewrite", "execute", "failed"] {
        assert!(
            validate_blog_call(
                action,
                json!({"id":"article","revision":1,"model":"fixture"})
            )
            .is_err(),
            "{action}"
        );
    }
    assert!(
        validate_catalog_call(
            &blog_catalog(),
            &json!({"extension":"akra.tech-blog","version":1,"action":"catalog","input":{}})
        )
        .is_err()
    );
}

#[test]
fn blog_v2_catalog_preserves_the_public_surface_and_saved_v1_outputs() {
    let operations = crate::domain::repository_extension::tech_blog::operations();
    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.name.as_str())
            .collect::<Vec<_>>(),
        [
            "catalog",
            "configure",
            "targets",
            "list",
            "open",
            "seen",
            "generate",
            "cancel",
            "resume",
            "revise",
            "review",
            "save",
            "restore",
            "figure",
            "export",
            "delete"
        ]
    );
    // Python present() adds v2 display fields without migrating the saved v1 article.
    let legacy = json!({
        "id":"v1","version":1,"topic":"old","title":"old","draft":"saved","revision":3,
        "job":null,"review":{"state":"reviewed","findings":[]},"figures":[],"messages":[],
        "state":"reviewed","stage":null,"stopped":null,"commits":[],"evidence":null,"calls":0,
        "edition":1,"previous":null,"unread":false
    });
    for action in ["open", "seen", "cancel", "save", "figure", "export"] {
        let operation = operations
            .iter()
            .find(|entry| entry.name == action)
            .unwrap();
        assert!(
            jsonschema::validator_for(&operation.output_schema)
                .unwrap()
                .is_valid(&legacy),
            "{action}"
        );
    }
    let list = operations
        .iter()
        .find(|entry| entry.name == "list")
        .unwrap();
    assert!(
        jsonschema::validator_for(&list.output_schema)
            .unwrap()
            .is_valid(&json!([{
                "id":"v1","title":"old","state":"reviewed",
                "stage":null,"task":null,"updated":null,"unread":false,"failed":false
            }]))
    );
}

mod storage_desktop_fixture;
