use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::application::port::outbound::mcp_hub_backend_port::McpHubBackendPort;
use crate::domain::mcp_hub::{McpHubServerStatus, McpHubToolInfo};
use serde_json::json;

static NEXT_HOME: AtomicU64 = AtomicU64::new(0);

fn backend() -> CodexMcpHubBackend {
    let home = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-catalog-{}-{}",
        std::process::id(),
        NEXT_HOME.fetch_add(1, Ordering::Relaxed)
    ));
    let backend = CodexMcpHubBackend::with_home(home);
    backend.write_document(&"[mcp_servers.one]\nurl='https://one.example/mcp'\n[mcp_servers.two]\nurl='https://two.example/mcp'\n".parse().unwrap()).unwrap();
    backend
}

fn status(name: &str) -> McpHubServerStatus {
    McpHubServerStatus {
        name: name.into(),
        auth: "oAuth".into(),
        connection: "reachable".into(),
        error: None,
        tools: vec![McpHubToolInfo {
            name: "read".into(),
            description: "Full description\nwith detail".into(),
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"}}}),
        }],
    }
}

#[test]
fn warm_catalogs_survive_backend_restart_without_discovery() {
    let first = backend();
    let mut called = Vec::new();
    let expected = first
        .catalogs_at(1000, |name, _| {
            called.push(name.to_owned());
            Ok(vec![status(name)])
        })
        .unwrap();
    assert_eq!(called, ["one", "two"]);
    let second = CodexMcpHubBackend::with_home(first.home.clone());
    let actual = second
        .catalogs_at(1001, |_, _| panic!("warm search must not start discovery"))
        .unwrap();
    assert_eq!(actual, expected);
    std::fs::remove_dir_all(&first.home).unwrap();
}

#[test]
fn only_the_missing_server_is_discovered_and_failed_discovery_is_not_cached() {
    let backend = backend();
    backend
        .catalogs_at(1000, |name, _| {
            let mut result = status(name);
            if name == "two" {
                result.error = Some("unavailable".into());
                result.tools.clear();
            }
            Ok(vec![result])
        })
        .unwrap();
    let mut called = Vec::new();
    backend
        .catalogs_at(1001, |name, _| {
            called.push(name.to_owned());
            Ok(vec![status(name)])
        })
        .unwrap();
    assert_eq!(called, ["two"]);
    std::fs::remove_dir_all(&backend.home).unwrap();
}

#[test]
fn empty_success_is_cached_but_authentication_failure_is_not() {
    let backend = backend();
    backend
        .catalogs_at(1000, |name, _| {
            let mut result = status(name);
            result.tools.clear();
            if name == "two" {
                result.auth = "notLoggedIn".into();
            }
            Ok(vec![result])
        })
        .unwrap();
    let mut called = Vec::new();
    backend
        .catalogs_at(1001, |name, _| {
            called.push(name.to_owned());
            Ok(vec![status(name)])
        })
        .unwrap();
    assert_eq!(called, ["two"]);
    std::fs::remove_dir_all(&backend.home).unwrap();
}

#[test]
fn expiry_and_explicit_invalidation_trigger_fresh_discovery() {
    let backend = backend();
    backend
        .catalogs_at(1000, |name, _| Ok(vec![status(name)]))
        .unwrap();
    backend
        .catalogs_at(1599, |_, _| panic!("still fresh"))
        .unwrap();
    let mut called = Vec::new();
    backend
        .catalogs_at(1600, |name, _| {
            called.push(name.to_owned());
            Ok(vec![status(name)])
        })
        .unwrap();
    assert_eq!(called, ["one", "two"]);
    backend.invalidate_tool_catalogs().unwrap();
    called.clear();
    backend
        .catalogs_at(1601, |name, _| {
            called.push(name.to_owned());
            Ok(vec![status(name)])
        })
        .unwrap();
    assert_eq!(called, ["one", "two"]);
    std::fs::remove_dir_all(&backend.home).unwrap();
}

#[test]
fn configuration_changes_exclude_disabled_and_removed_servers() {
    let backend = backend();
    backend
        .catalogs_at(1000, |name, _| Ok(vec![status(name)]))
        .unwrap();
    backend
        .write_document(
            &"[mcp_servers.one]\nurl='https://one.example/mcp'\nenabled=false\n"
                .parse()
                .unwrap(),
        )
        .unwrap();
    let catalogs = backend
        .catalogs_at(1001, |_, _| panic!("disabled server"))
        .unwrap();
    assert!(catalogs.is_empty());
    std::fs::remove_dir_all(&backend.home).unwrap();
}

#[test]
fn a_concurrent_invalidation_cannot_restore_old_entries() {
    let backend = backend();
    backend
        .catalogs_at(1000, |name, _| {
            backend.invalidate_tool_catalogs().unwrap();
            Ok(vec![status(name)])
        })
        .unwrap();
    let mut called = Vec::new();
    backend
        .catalogs_at(1001, |name, _| {
            called.push(name.to_owned());
            Ok(vec![status(name)])
        })
        .unwrap();
    assert_eq!(called, ["one", "two"]);
    std::fs::remove_dir_all(&backend.home).unwrap();
}

#[test]
fn cache_failure_does_not_hide_successful_live_discovery() {
    let backend = backend();
    std::fs::write(backend.home.join("tool-catalog.sqlite"), b"not sqlite").unwrap();
    let catalogs = backend
        .catalogs_at(1000, |name, _| Ok(vec![status(name)]))
        .unwrap();
    assert_eq!(catalogs.len(), 2);
    std::fs::remove_dir_all(&backend.home).unwrap();
}

#[test]
fn identity_covers_extra_configuration_and_credential_environment_without_plaintext() {
    let document: DocumentMut = "[mcp_servers.one]\nurl='https://one.example/mcp'\nbearer_token_env_var='MY_TOKEN'\nenv_vars=['OTHER_TOKEN']\n".parse().unwrap();
    let first = fingerprint_with_environment(&document, |name| {
        (name == "MY_TOKEN").then(|| "secret-one".into())
    });
    let second = fingerprint_with_environment(&document, |name| {
        (name == "MY_TOKEN").then(|| "secret-two".into())
    });
    assert_ne!(first, second);
    assert!(!first.contains("secret"));
    let altered: DocumentMut = format!("{document}startup_timeout_sec=90\n")
        .parse()
        .unwrap();
    assert_ne!(
        first,
        fingerprint_with_environment(&altered, |name| (name == "MY_TOKEN")
            .then(|| "secret-one".into()))
    );
}

#[test]
fn targeted_discovery_uses_the_official_thread_scope_on_the_wire() {
    let backend = backend();
    let script = backend.home.join("fake-app-server.cjs");
    std::fs::write(&script, r#"
const readline = require('node:readline');
readline.createInterface({input:process.stdin}).on('line', line => {
  const m = JSON.parse(line);
  if (m.id === undefined) return;
  let result = {};
  if (m.method === 'thread/start') {
    if (m.params.config['mcp_servers.two.enabled'] !== false || m.params.ephemeral !== true)
      return process.stdout.write(JSON.stringify({id:m.id,error:{message:'other server enabled'}})+'\n');
    result = {thread:{id:'scoped-one'}};
  }
  if (m.method === 'mcpServerStatus/list') {
    if (m.params.threadId !== 'scoped-one' || m.params.detail !== 'toolsAndAuthOnly' || 'serverName' in m.params)
      return process.stdout.write(JSON.stringify({id:m.id,error:{message:'missing official thread scope'}})+'\n');
    result = {data:[{name:'one',tools:{read:{name:'read',inputSchema:{type:'object'}}}}],nextCursor:null};
  }
  process.stdout.write(JSON.stringify({id:m.id,result})+'\n');
});
"#).unwrap();
    let node = which::which("node").unwrap();
    let command = crate::trusted_executable::TrustedCommand {
        program: node.clone(),
        source_executable: node,
        prefix_args: vec![script.into_os_string()],
        trust_warning: None,
    };
    let mut session =
        super::super::session::HubAppServerSession::open(&command, &backend.home).unwrap();
    let mut threads = std::collections::HashMap::new();
    let thread = backend
        .thread_for_server(&mut session, &mut threads, "one")
        .unwrap();
    let statuses = statuses_from(&mut session, Some(&thread)).unwrap();
    assert_eq!(statuses[0].name, "one");
    assert_eq!(statuses[0].tools[0].name, "read");
    drop(session);
    std::fs::remove_dir_all(&backend.home).unwrap();
}
