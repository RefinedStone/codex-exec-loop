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

#[tokio::test]
async fn legacy_blog_job_success_is_returned_without_retry_or_catalog_mismatch() {
    // Fake worker responses exercise both model-job entrances without starting any model turn.
    let catalog = json!([{
        "id":"akra.tech-blog","version":1,
        "operations":crate::domain::repository_extension::tech_blog::operations()
    }]);
    for action in ["write", "review"] {
        let result = json!({"id":"article","revision":2,"job":{"id":"already-started"}});
        let answer = call_with_catalog_response(
            catalog.clone(),
            json!({"extension":"akra.tech-blog","version":1,"action":action,
                "input":{"id":"article","revision":1,"model":"fixture"}}),
            json!({"result":result}),
        )
        .await
        .unwrap();
        assert_eq!(answer, result);
    }
}

#[test]
fn legacy_catalog_matches_optional_defaults_and_required_worker_fields() {
    let catalog = json!([{
        "id":"akra.tech-blog","version":1,
        "operations":crate::domain::repository_extension::tech_blog::operations()
    }]);
    for (action, input) in [
        (
            "create",
            json!({"topic":"topic","style":"PROBLEM_SOLVING","mode":"PROJECT_RETROSPECTIVE"}),
        ),
        ("save", json!({"id":"article","revision":1})),
        (
            "write",
            json!({"id":"article","revision":1,"model":"fixture"}),
        ),
        (
            "review",
            json!({"id":"article","revision":1,"model":"fixture","instruction":""}),
        ),
    ] {
        validate_catalog_call(
            &catalog,
            &json!({"extension":"akra.tech-blog","version":1,"action":action,"input":input}),
        )
        .unwrap();
    }
    for (action, input) in [
        ("configure", json!({"enabled":1})),
        (
            "create",
            json!({"topic":"topic","style":"unknown","mode":"TECH_EXPLORATION"}),
        ),
        (
            "create",
            json!({"topic":"topic","style":"PROBLEM_SOLVING","mode":"TECH_EXPLORATION",
            "paths":["1","2","3","4","5"]}),
        ),
        (
            "write",
            json!({"id":"article","revision":1,"model":"bad model"}),
        ),
        ("save", json!({"id":"article","revision":1,"draft":1})),
    ] {
        assert!(
            validate_catalog_call(
                &catalog,
                &json!({"extension":"akra.tech-blog","version":1,"action":action,"input":input})
            )
            .is_err()
        );
    }
}
