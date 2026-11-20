use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::json;

use super::HubAppServerSession;
use crate::trusted_executable::TrustedCommand;

fn fake_session(stall: bool) -> (HubAppServerSession, PathBuf) {
    let home = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-hub-deadline-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&home).unwrap();
    let script = home.join("fake-app-server.cjs");
    std::fs::write(
        &script,
        r#"
const readline = require('node:readline');
const lines = readline.createInterface({input:process.stdin});
lines.on('line', line => {
  const message = JSON.parse(line);
  if (message.method === 'initialized' && process.argv[2] === 'stall') {
    lines.pause();
    process.stdin.pause();
    // The stdout queue fills while the RPC writer is blocked on unread stdin.
    setInterval(() => {
      process.stdout.write('{"method":"unrelated","params":{}}\n');
    }, 1);
    return;
  }
  if (message.id !== undefined)
    process.stdout.write(JSON.stringify({id:message.id,result:{ok:true}}) + '\n');
});
"#,
    )
    .unwrap();
    let node = which::which("node").unwrap();
    let command = TrustedCommand {
        program: node.clone(),
        source_executable: node,
        prefix_args: vec![
            script.into_os_string(),
            if stall { "stall" } else { "healthy" }.into(),
        ],
        trust_warning: None,
    };
    (HubAppServerSession::open(&command, &home).unwrap(), home)
}

#[test]
fn blocked_stdin_with_stdout_notifications_times_out_and_a_fresh_session_works() {
    let (mut stalled, home) = fake_session(true);
    let began = Instant::now();
    let error = stalled
        .request(
            "large-request",
            json!({"payload": "x".repeat(2 * 1024 * 1024)}),
            Duration::from_millis(500),
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("timed out"), "{error:#}");
    assert!(began.elapsed() < Duration::from_secs(4), "{error:#}");
    let teardown = Instant::now();
    drop(stalled);
    assert!(teardown.elapsed() < Duration::from_secs(4));
    let _ = std::fs::remove_dir_all(home);

    let (mut healthy, home) = fake_session(false);
    let result = healthy
        .request("echo", json!({}), Duration::from_secs(5))
        .unwrap();
    assert_eq!(result["ok"], true);
    drop(healthy);
    let _ = std::fs::remove_dir_all(home);
}
