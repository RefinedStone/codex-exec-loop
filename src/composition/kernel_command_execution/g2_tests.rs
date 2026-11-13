//! A model-free client learns the command and its recovery steps from describe.

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::DuplexStream;
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, protocol::Role},
};

use crate::adapter::inbound::kernel_protocol::{
    CommandRequest, CommandResponse, KernelCommand, KernelEvent as ProtocolEvent,
    KernelQueryAnswer, describe,
};
use crate::domain::kernel_event::{
    KernelEvent, OperationReceipt, OperationStatus, RecordedKernelEvent, TurnEndOutcome,
};
use crate::domain::kernel_spec::{CommandCompletion, ReasonCode};

async fn receive(socket: &mut WebSocketStream<DuplexStream>) -> Value {
    let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
        panic!("expected a JSON text frame");
    };
    serde_json::from_str(text.as_str()).unwrap()
}

async fn send(socket: &mut WebSocketStream<DuplexStream>, value: Value) {
    socket.send(Message::text(value.to_string())).await.unwrap();
}

async fn reply(socket: &mut WebSocketStream<DuplexStream>, request: &Value, value: Value) {
    send(
        socket,
        json!({"jsonrpc":"2.0","id":request["id"],"result":value}),
    )
    .await;
}

async fn call(
    socket: &mut WebSocketStream<DuplexStream>,
    id: i64,
    method: &str,
    params: Value,
) -> Value {
    send(
        socket,
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
    )
    .await;
    let response = receive(socket).await;
    assert_eq!(response["id"], id);
    assert!(response.get("error").is_none());
    response["result"].clone()
}

#[tokio::test]
async fn describe_only_client_follows_rejection_event_resend_and_operation_end() {
    let (client, server) = tokio::io::duplex(128 * 1024);
    let mut client = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
    let mut server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
    let serving = async {
        let request = receive(&mut server).await;
        assert_eq!(request["method"], "akra/kernel/describe");
        reply(
            &mut server,
            &request,
            serde_json::to_value(describe()).unwrap(),
        )
        .await;
        let request = receive(&mut server).await;
        assert_eq!(request["method"], "akra/kernel/attach");
        reply(&mut server, &request, json!({})).await;
        let request = receive(&mut server).await;
        let first: CommandRequest = serde_json::from_value(request["params"].clone()).unwrap();
        assert!(matches!(first.command, KernelCommand::SubmitPrompt { .. }));
        let rejection = CommandResponse::replayed(OperationReceipt {
            op_id: 1,
            rejection: Some(ReasonCode::Busy),
            replayed: false,
        });
        reply(
            &mut server,
            &request,
            serde_json::to_value(rejection).unwrap(),
        )
        .await;
        send(&mut server, json!({
            "jsonrpc":"2.0","method":"akra/kernel/event",
            "params":{"event":serde_json::to_value(ProtocolEvent::ThreadActivity { threads:Vec::new() }).unwrap()}
        })).await;
        let request = receive(&mut server).await;
        let resent: CommandRequest = serde_json::from_value(request["params"].clone()).unwrap();
        assert_eq!(first.command, resent.command);
        assert_ne!(first.idempotency_key, resent.idempotency_key);
        let accepted =
            CommandResponse::from_core(Some(&[])).with_operation(Some(OperationReceipt {
                op_id: 3,
                rejection: None,
                replayed: false,
            }));
        reply(
            &mut server,
            &request,
            serde_json::to_value(accepted).unwrap(),
        )
        .await;
        let request = receive(&mut server).await;
        assert_eq!(request["params"]["query"]["op_id"], 3);
        let recorded = |sequence, event| RecordedKernelEvent {
            sequence,
            recorded_at: "now".to_string(),
            event,
        };
        let records = [
            recorded(
                3,
                KernelEvent::CommandReceived {
                    op_id: 3,
                    command: resent.command.kind().name().to_string(),
                    thread_id: "thread".to_string(),
                    fingerprint: "fingerprint".to_string(),
                    idempotency_key: resent.idempotency_key,
                    completion: Some(CommandCompletion::Records),
                    caller: Some("spec-only-client".to_string()),
                },
            ),
            recorded(
                4,
                KernelEvent::TurnEnded {
                    thread_id: "thread".to_string(),
                    turn_id: "fake-turn".to_string(),
                    op_id: Some(3),
                    outcome: Some(TurnEndOutcome::Completed),
                },
            ),
        ];
        let answer = KernelQueryAnswer::Operation(OperationStatus::from_records(3, records));
        reply(&mut server, &request, serde_json::to_value(answer).unwrap()).await;
    };
    let consuming = async {
        let spec = call(&mut client, 1, "akra/kernel/describe", json!({})).await;
        let command = spec["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| {
                let properties = &entry["input"]["properties"];
                properties.get("prompt").is_some()
                    && properties.get("thread_id").is_some()
                    && properties.get("expected_turn_id").is_none()
            })
            .expect("describe identifies one thread prompt command");
        let query = spec["queries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["input"]["properties"].get("op_id").is_some())
            .expect("describe identifies the operation query");
        call(
            &mut client,
            2,
            "akra/kernel/attach",
            json!({
                "workspace":"/workspace","client_name":"spec-only-client","transient":true
            }),
        )
        .await;
        let input = json!({"type":command["name"],"thread_id":"thread","prompt":"a prompt"});
        let rejected = call(
            &mut client,
            3,
            "akra/kernel/command",
            json!({"command":input,"idempotency_key":"first"}),
        )
        .await;
        let mut resend = false;
        for step in rejected["rejection"]["next"].as_array().unwrap() {
            match step["step"].as_str().unwrap() {
                "wait_for" => {
                    let event = receive(&mut client).await;
                    assert_eq!(event["params"]["event"]["type"], step["event"]);
                }
                "resend" => resend = true,
                other => panic!("unexpected next step {other}"),
            }
        }
        assert!(resend);
        let accepted = call(
            &mut client,
            4,
            "akra/kernel/command",
            json!({"command":input,"idempotency_key":"second"}),
        )
        .await;
        assert!(accepted.get("rejection").is_none());
        let operation = call(
            &mut client,
            5,
            "akra/kernel/query",
            json!({
                "query":{"type":query["name"],"op_id":accepted["operation"]["op_id"]}
            }),
        )
        .await;
        assert_eq!(operation["state"], "done");
        assert_eq!(operation["records"].as_array().unwrap().len(), 2);
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(serving, consuming);
    })
    .await
    .expect("the described event and response must finish without polling");
}
