use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde_json::json;

use super::HubAppServerSession;
use crate::trusted_executable::TrustedCommand;

const HELPER: &str =
    "adapter::outbound::mcp_hub::session::environment_tests::hub_environment_subprocess_helper";

#[test]
fn hub_environment_subprocess_helper() {
    let Some(home) = std::env::var_os("AKRA_TEST_MCP_ENV_HOME") else {
        return;
    };
    let home = PathBuf::from(home);
    let node = which::which("node").unwrap();
    let command = TrustedCommand {
        program: node.clone(),
        source_executable: node,
        prefix_args: vec![home.join("fake-app-server.cjs").into_os_string()],
        trust_warning: None,
    };
    let mut session = HubAppServerSession::open(&command, &home).unwrap();
    let actual = session
        .request("environment/read", json!({}), Duration::from_secs(5))
        .unwrap();
    assert_eq!(actual["token"], "selected-token");
    assert_eq!(actual["stdio"], "selected-stdio");
    assert_eq!(actual["header"], "selected-header");
    assert_eq!(PathBuf::from(actual["home"].as_str().unwrap()), home);
    for name in [
        "AWS_SECRET_ACCESS_KEY",
        "GH_TOKEN",
        "OPENAI_API_KEY",
        "AKRA_TEST_MCP_ENV_HOME",
    ] {
        assert!(
            actual[name].is_null(),
            "{name} was not selected by the hub config"
        );
    }
}

#[test]
fn hub_open_preserves_only_configuration_selected_environment() {
    let home = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-hub-env-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&home).unwrap();
    // Windows must match a mixed-case environment key without changing the policy on Unix.
    let token_name = if cfg!(windows) {
        "akra_mcp_selected_token"
    } else {
        "AKRA_MCP_SELECTED_TOKEN"
    };
    let config = format!(
        "[mcp_servers.http]\nurl = 'https://example.test/mcp'\nbearer_token_env_var = '{token_name}'\n\
         [mcp_servers.http.env_http_headers]\nAuthorization = 'AKRA_MCP_SELECTED_HEADER'\n\
         [mcp_servers.stdio]\ncommand = 'unused'\nenv_vars = ['AKRA_MCP_SELECTED_STDIO']\n"
    );
    std::fs::write(home.join("config.toml"), config).unwrap();
    std::fs::write(
        home.join("fake-app-server.cjs"),
        r#"
const readline = require('node:readline');
readline.createInterface({ input: process.stdin }).on('line', line => {
  const message = JSON.parse(line);
  if (message.id === undefined) return;
  const result = message.method === 'environment/read' ? {
    token: process.env.AKRA_MCP_SELECTED_TOKEN,
    stdio: process.env.AKRA_MCP_SELECTED_STDIO,
    header: process.env.AKRA_MCP_SELECTED_HEADER,
    home: process.env.CODEX_HOME,
    AWS_SECRET_ACCESS_KEY: process.env.AWS_SECRET_ACCESS_KEY,
    GH_TOKEN: process.env.GH_TOKEN,
    OPENAI_API_KEY: process.env.OPENAI_API_KEY,
    AKRA_TEST_MCP_ENV_HOME: process.env.AKRA_TEST_MCP_ENV_HOME
  } : {};
  process.stdout.write(JSON.stringify({ id: message.id, result }) + '\n');
});
"#,
    )
    .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", HELPER, "--nocapture"])
        .env("AKRA_TEST_MCP_ENV_HOME", &home)
        .env("AKRA_APP_SERVER_PROCESS_ENVIRONMENT", "scrubbed")
        .env("AKRA_APP_SERVER_API_KEY_AUTH", "0")
        .env("AKRA_MCP_SELECTED_TOKEN", "selected-token")
        .env("AKRA_MCP_SELECTED_STDIO", "selected-stdio")
        .env("AKRA_MCP_SELECTED_HEADER", "selected-header")
        .env("AWS_SECRET_ACCESS_KEY", "unselected-aws-canary")
        .env("GH_TOKEN", "unselected-github-canary")
        .env("OPENAI_API_KEY", "unselected-openai-canary")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(home);
    assert!(
        output.status.success(),
        "hub environment subprocess failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
