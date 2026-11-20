use std::path::PathBuf;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use serde_json::json;

use super::HubAppServerSession;
use crate::trusted_executable::TrustedCommand;

fn fake_session() -> (HubAppServerSession, PathBuf) {
    let home = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-hub-transport-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&home).unwrap();
    let script = home.join("fake-app-server.cjs");
    std::fs::write(
        &script,
        r#"
const readline = require('node:readline');
const fs = require('node:fs');
const send = message => process.stdout.write(JSON.stringify(message) + '\n');
readline.createInterface({ input: process.stdin }).on('line', line => {
  const message = JSON.parse(line);
  if (message.method === 'initialized') {
    for (let i = 0; i < 24; i++) send({method:'mcpServer/startupStatus/updated',params:{i}});
    return;
  }
  if (message.id === undefined) return;
  if (message.method === 'long-stderr') {
    process.stderr.write('x'.repeat(1024 * 1024) + '\n', () => {
      send({id:message.id,result:{drained:true}});
    });
    return;
  }
  if (message.method === 'stdout-eof') {
    fs.closeSync(1);
    return;
  }
  if (message.method === 'last-response') {
    fs.writeSync(1, JSON.stringify({id:message.id,result:{last:true}}) + '\n');
    fs.closeSync(1);
    return;
  }
  send({id:message.id,result:{ok:true}});
});
"#,
    )
    .unwrap();
    let node = which::which("node").unwrap();
    let command = TrustedCommand {
        program: node.clone(),
        source_executable: node,
        prefix_args: vec![script.into_os_string()],
        trust_warning: None,
    };
    (HubAppServerSession::open(&command, &home).unwrap(), home)
}

#[test]
fn notification_burst_and_long_stderr_do_not_break_next_rpc() {
    let (mut session, home) = fake_session();
    let result = session
        .request("echo", json!({}), Duration::from_secs(5))
        .unwrap();
    assert_eq!(result["ok"], true);
    let result = session
        .request("long-stderr", json!({}), Duration::from_secs(5))
        .unwrap();
    assert_eq!(result["drained"], true);
    assert!(
        session
            .stderr_tail
            .lock()
            .unwrap()
            .iter()
            .all(|line| line.len() <= super::MAX_STDERR_LINE_BYTES)
    );
    drop(session);
    let _ = std::fs::remove_dir_all(home);
}

#[cfg(unix)]
#[test]
fn stdout_eof_fails_promptly_while_stdin_and_stderr_stay_open() {
    let (mut session, home) = fake_session();
    let began = Instant::now();
    let error = session
        .request("stdout-eof", json!({}), Duration::from_secs(10))
        .unwrap_err();
    assert!(began.elapsed() < Duration::from_secs(3), "{error:#}");
    assert!(error.to_string().contains("exited"), "{error:#}");
    drop(session);
    let _ = std::fs::remove_dir_all(home);
}

#[cfg(unix)]
#[test]
fn complete_response_before_stdout_eof_is_delivered() {
    let (mut session, home) = fake_session();
    let result = session
        .request("last-response", json!({}), Duration::from_secs(5))
        .unwrap();
    assert_eq!(result["last"], true);
    let began = Instant::now();
    let error = session
        .request("after-eof", json!({}), Duration::from_secs(10))
        .unwrap_err();
    assert!(began.elapsed() < Duration::from_secs(3), "{error:#}");
    drop(session);
    let _ = std::fs::remove_dir_all(home);
}
