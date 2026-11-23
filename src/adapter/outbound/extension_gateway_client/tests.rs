use super::*;
use tokio_tungstenite::tungstenite::protocol::Role;

async fn call_with_server_response(response: Value) -> Result<Value> {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let mut client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
    let mut server = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
    let task = tokio::spawn(async move {
        for step in 0..3 {
            let message = server.next().await.unwrap().unwrap();
            let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            let mut answer = json!({"jsonrpc":"2.0","id":request["id"]});
            match step {
                0 => answer["result"] = json!({"kernel":"test-local-host"}),
                1 => {
                    answer["result"] = json!([{
                        "id":"akra.example","version":1,"operations":[{
                            "name":"echo","input_schema":{
                                "type":"object","required":["text"],"properties":{"text":{"type":"string"}},
                                "additionalProperties":false
                            }
                        }]
                    }])
                }
                _ => {
                    assert_eq!(request["method"], "akra/kernel/extensions/call");
                    assert_eq!(request["params"]["input"], json!({"text":1}));
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
    });
    let answer = request_over_socket(
        &mut client,
        "local_user",
        "",
        Some(json!({
            "extension":"akra.example","version":1,"action":"echo","input":{"text":1}
        })),
    )
    .await;
    task.await.unwrap();
    answer
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
