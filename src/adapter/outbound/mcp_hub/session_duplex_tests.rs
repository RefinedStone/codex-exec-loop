use std::time::Duration;

use serde_json::json;

use super::HubAppServerSession;
use crate::trusted_executable::TrustedCommand;

fn run_request_after_startup_burst(mode: &str) {
    let home = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-hub-duplex-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&home).unwrap();
    let script = home.join("fake-app-server.cjs");
    std::fs::write(
        &script,
        r#"
const fs = require('node:fs');
const readline = require('node:readline');
const mode = process.argv[2];
const reverse = mode.startsWith('reverse');
const pause = new Int32Array(new SharedArrayBuffer(4));
function writeAll(bytes) {
  let offset = 0;
  while (offset < bytes.length) {
    try {
      offset += fs.writeSync(1, bytes, offset, bytes.length - offset);
    } catch (error) {
      if (error.code !== 'EAGAIN') throw error;
      Atomics.wait(pause, 0, 0, 1);
    }
  }
}
function send(message) {
  writeAll(Buffer.from(JSON.stringify(message) + '\n'));
}
function burst() {
  const notification = Buffer.from(JSON.stringify({
    method: 'startup-notification',
    params: {padding: 'x'.repeat(4096)}
  }) + '\n');
  for (let index = 0; index < 256; index++) writeAll(notification);
}
const lines = readline.createInterface({input: process.stdin});
lines.on('line', line => {
  const message = JSON.parse(line);
  if (message.method === 'initialized') {
    if (reverse) {
      const count = mode === 'reverse-two' ? 2 : 1;
      for (let index = 0; index < count; index++) {
        // A large id makes each reverse reply exceed pipe capacity as well.
        send({
          id: index + 'r'.repeat(2 * 1024 * 1024),
          method: 'mcpServer/elicitation/request',
          params: {}
        });
      }
    }
    // More than the bounded channel and OS pipe can hold. Finish this burst
    // before returning to the event loop to read the large RPC.
    burst();
    return;
  }
  if (message.id !== undefined && message.method) {
    send({id: message.id, result: {received: message.params?.payload?.length ?? 0, authorizationUrl: 'https://example.invalid/oauth'}});
    if (mode === 'reverse-login' && message.method === 'mcpServer/oauth/login') {
      send({method: 'mcpServer/oauthLogin/completed', params: {name: 'other-server', success: false}});
      send({method: 'mcpServer/oauthLogin/completed', params: {name: 'expected-server', success: true}});
    }
    if (reverse && ['large-request', 'mcpServer/oauth/login'].includes(message.method)) {
      if (mode === 'reverse-eof') {
        lines.pause();
        process.stdin.pause();
        fs.closeSync(1);
        setTimeout(() => { lines.resume(); process.stdin.resume(); }, 100);
      } else {
        // The response arrives before the child drains reverse replies.
        // Waiting for their ACK without reading this burst would deadlock again.
        burst();
      }
    }
  }
});
"#,
    )
    .unwrap();
    let node = which::which("node").unwrap();
    let command = TrustedCommand {
        program: node.clone(),
        source_executable: node,
        prefix_args: vec![script.into_os_string(), mode.into()],
        trust_warning: None,
    };
    let mut session = HubAppServerSession::open(&command, &home).unwrap();
    let payload_bytes = 2 * 1024 * 1024;
    let result = if mode == "reverse-login" {
        session
            .request_with_notification(
                "mcpServer/oauth/login",
                json!({"name": "expected-server", "payload": "x".repeat(payload_bytes)}),
                Duration::from_secs(5),
                "mcpServer/oauthLogin/completed",
                |params| params["name"] == "expected-server",
            )
            .and_then(|(response, captured)| {
                let captured_before_return = captured.is_some();
                let completed = match captured {
                    Some(params) => params,
                    None => session.wait_for_notification(
                        "mcpServer/oauthLogin/completed",
                        Duration::from_secs(2),
                        |params| params["name"] == "expected-server",
                    )?,
                };
                assert!(
                    captured_before_return,
                    "completion must survive post-response drain"
                );
                assert_eq!(completed["success"], true);
                assert_eq!(completed["name"], "expected-server");
                Ok(response)
            })
    } else {
        session.request(
            "large-request",
            json!({"payload": "x".repeat(payload_bytes)}),
            Duration::from_secs(5),
        )
    };
    drop(session);
    let _ = std::fs::remove_dir_all(home);
    let response =
        result.unwrap_or_else(|error| panic!("healthy duplex mode {mode} must finish: {error:#}"));
    assert_eq!(response["received"], payload_bytes);
}

#[test]
fn large_request_succeeds_after_blocking_startup_notification_burst() {
    run_request_after_startup_burst("normal");
}

#[test]
fn reverse_reply_keeps_draining_after_the_matching_response() {
    run_request_after_startup_burst("reverse");
}

#[test]
fn multiple_reverse_replies_do_not_exhaust_the_writer_channel() {
    run_request_after_startup_burst("reverse-two");
}

#[cfg(unix)]
#[test]
fn complete_response_survives_stdout_eof_while_reverse_reply_finishes() {
    run_request_after_startup_burst("reverse-eof");
}

#[test]
fn pending_replies_preserve_capacity_and_consume_each_acknowledgement_once() {
    let (writer, receiver) = std::sync::mpsc::sync_channel(1);
    let mut pending = super::PendingWrites::default();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    pending
        .enqueue(&writer, &json!({"id": 1}), deadline)
        .unwrap();
    pending
        .enqueue(&writer, &json!({"id": 2}), deadline)
        .unwrap();
    pending
        .enqueue(&writer, &json!({"id": 3}), deadline)
        .unwrap();
    for _ in 0..3 {
        let request = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        request.response.send(Ok(())).unwrap();
        pending.progress(&writer, deadline).unwrap();
    }
    assert!(pending.is_empty());
    pending.progress(&writer, deadline).unwrap();
}

#[test]
fn pending_reply_failure_is_not_hidden_by_a_successful_initial_write() {
    let (writer, receiver) = std::sync::mpsc::sync_channel(1);
    let mut pending = super::PendingWrites::default();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    pending
        .enqueue(&writer, &json!({"id": 1}), deadline)
        .unwrap();
    pending
        .enqueue(&writer, &json!({"id": 2}), deadline)
        .unwrap();
    receiver.recv().unwrap().response.send(Ok(())).unwrap();
    pending.progress(&writer, deadline).unwrap();
    receiver
        .recv()
        .unwrap()
        .response
        .send(Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "reverse write failed",
        )))
        .unwrap();
    let error = pending.progress(&writer, deadline).unwrap_err();
    assert!(error.to_string().contains("reverse write failed"));
}

#[test]
fn pending_control_replies_have_a_fixed_memory_bound() {
    let (writer, _receiver) = std::sync::mpsc::sync_channel(1);
    let mut pending = super::PendingWrites::default();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    for id in 0..super::MAX_PENDING_WRITES {
        pending
            .enqueue(&writer, &json!({"id": id}), deadline)
            .unwrap();
    }
    let error = pending
        .enqueue(&writer, &json!({"id": "overflow"}), deadline)
        .unwrap_err();
    assert!(error.to_string().contains("write queue is full"));
    assert_eq!(pending.writes.len(), super::MAX_PENDING_WRITES);
}

#[test]
fn oauth_completion_is_retained_while_reverse_replies_finish() {
    run_request_after_startup_burst("reverse-login");
}
