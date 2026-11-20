//! Exercise the real file replacement, local resident, and CLI-to-Gateway boundary.
//! The fake app-server only acknowledges protocol initialization; it starts no model turns.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{Value, json};

use super::gateway_client::{GatewayHubTransport, GatewayReply, GatewayRoutedMcpHubBackend};
use super::registry;
use super::session::HubAppServerSession;
use super::tool_cache::ToolCatalogCache;
use super::{CONFIG_FILE, CodexMcpHubBackend, ResidentEngine, SKILLS_DIRECTORY};
use crate::application::port::outbound::mcp_hub_backend_port::McpHubBackendPort;
use crate::domain::mcp_hub::{MCP_HUB_GATEWAY_RELOAD, McpHubServerSpec};
use crate::trusted_executable::TrustedCommand;

struct RecordingGateway {
    reloads: Arc<AtomicUsize>,
    fail: Arc<AtomicBool>,
}

impl GatewayHubTransport for RecordingGateway {
    fn request(&self, method: &str, _params: Value, _timeout: Duration) -> Result<GatewayReply> {
        assert_eq!(method, MCP_HUB_GATEWAY_RELOAD);
        self.reloads.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            anyhow::bail!("injected Gateway reload failure");
        }
        Ok(GatewayReply::Done(json!({})))
    }
}

struct Fixture {
    routed: GatewayRoutedMcpHubBackend,
    local: Arc<CodexMcpHubBackend>,
    reloads: Arc<AtomicUsize>,
    gateway_failure: Arc<AtomicBool>,
    home: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.local.reload();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn fixture(document: &str, sync: fn(&Path) -> io::Result<()>) -> Fixture {
    let home = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-registry-durability-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    crate::private_directory::ensure_private_directory(&home).unwrap();
    std::fs::write(home.join(CONFIG_FILE), document).unwrap();
    let script = home.join("fake-app-server.cjs");
    std::fs::write(
        &script,
        r#"
const readline = require('node:readline');
readline.createInterface({ input: process.stdin }).on('line', line => {
  const message = JSON.parse(line);
  if (message.id !== undefined) {
    process.stdout.write(JSON.stringify({ id: message.id, result: {} }) + '\n');
  }
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
    let session = HubAppServerSession::open(&command, &home).unwrap();
    let mut local = CodexMcpHubBackend::with_home(home.clone());
    local.sync_registry_directory = sync;
    local.resident = Some(Mutex::new(Some(ResidentEngine {
        session,
        threads: HashMap::new(),
        last_used: Instant::now(),
        catalog_identity: None,
    })));
    let local = Arc::new(local);
    let reloads = Arc::new(AtomicUsize::new(0));
    let gateway_failure = Arc::new(AtomicBool::new(false));
    let routed = GatewayRoutedMcpHubBackend::with_transport(
        local.clone(),
        Box::new(RecordingGateway {
            reloads: reloads.clone(),
            fail: gateway_failure.clone(),
        }),
    );
    Fixture {
        routed,
        local,
        reloads,
        gateway_failure,
        home,
    }
}

fn failing_directory_sync(home: &Path) -> io::Result<()> {
    assert!(home.join(CONFIG_FILE).is_file());
    Err(io::Error::other("injected registry directory sync failure"))
}

fn server() -> McpHubServerSpec {
    McpHubServerSpec::http("example", "https://example.test/mcp").unwrap()
}

fn has_resident(fixture: &Fixture) -> bool {
    fixture
        .local
        .resident
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .is_some()
}

fn assert_reconciled(fixture: &Fixture, previous_generation: i64) {
    assert!(!has_resident(fixture));
    assert_eq!(
        ToolCatalogCache::new(&fixture.home).generation().unwrap(),
        previous_generation + 1
    );
    assert_eq!(fixture.reloads.load(Ordering::SeqCst), 1);
}

#[test]
fn registration_reports_uncertain_durability_and_reloads_local_and_gateway() {
    let fixture = fixture("", failing_directory_sync);
    let generation = ToolCatalogCache::new(&fixture.home).generation().unwrap();

    let error = fixture.routed.register_server(&server()).unwrap_err();

    assert!(registry::was_replaced(&error));
    let detail = format!("{error:#}");
    assert!(
        detail.contains("durability after a crash is uncertain"),
        "{detail}"
    );
    assert!(
        detail.contains("injected registry directory sync failure"),
        "{detail}"
    );
    assert_eq!(fixture.local.registered_servers().unwrap(), [server()]);
    assert_reconciled(&fixture, generation);
}

#[test]
fn removal_reports_uncertain_durability_and_reloads_local_and_gateway() {
    let fixture = fixture(
        "[mcp_servers.example]\nurl = 'https://example.test/mcp'\n",
        failing_directory_sync,
    );
    let generation = ToolCatalogCache::new(&fixture.home).generation().unwrap();

    let error = fixture.routed.unregister_server("example").unwrap_err();

    assert!(registry::was_replaced(&error));
    assert!(format!("{error:#}").contains("injected registry directory sync failure"));
    assert!(fixture.local.registered_servers().unwrap().is_empty());
    assert_reconciled(&fixture, generation);
}

#[test]
fn rejected_update_keeps_the_document_catalog_and_resident_without_gateway_reload() {
    let document = "mcp_servers = 'not a table'\n";
    let fixture = fixture(document, failing_directory_sync);
    let generation = ToolCatalogCache::new(&fixture.home).generation().unwrap();

    let error = fixture.routed.register_server(&server()).unwrap_err();

    assert!(!registry::was_replaced(&error));
    assert_eq!(
        std::fs::read_to_string(fixture.home.join(CONFIG_FILE)).unwrap(),
        document
    );
    assert_eq!(
        ToolCatalogCache::new(&fixture.home).generation().unwrap(),
        generation
    );
    assert!(has_resident(&fixture));
    assert_eq!(fixture.reloads.load(Ordering::SeqCst), 0);
}

#[test]
fn removing_an_absent_server_does_not_invalidate_or_reload() {
    let fixture = fixture("", failing_directory_sync);
    let generation = ToolCatalogCache::new(&fixture.home).generation().unwrap();

    assert!(!fixture.routed.unregister_server("absent").unwrap());

    assert_eq!(
        ToolCatalogCache::new(&fixture.home).generation().unwrap(),
        generation
    );
    assert!(has_resident(&fixture));
    assert_eq!(fixture.reloads.load(Ordering::SeqCst), 0);
}

#[test]
fn skill_failure_after_replacement_still_invalidates_and_reloads() {
    let fixture = fixture("", registry::sync_directory);
    let generation = ToolCatalogCache::new(&fixture.home).generation().unwrap();
    std::fs::write(
        fixture.home.join(SKILLS_DIRECTORY),
        "blocks the skill directory",
    )
    .unwrap();

    let error = fixture.routed.register_server(&server()).unwrap_err();

    assert!(registry::was_replaced(&error));
    assert!(format!("{error:#}").contains("hub skill"));
    assert_eq!(fixture.local.registered_servers().unwrap(), [server()]);
    assert_reconciled(&fixture, generation);
}

#[test]
fn cache_failure_after_replacement_still_reloads_local_and_gateway() {
    let fixture = fixture("", registry::sync_directory);
    std::fs::create_dir(fixture.home.join("tool-catalog.sqlite")).unwrap();

    let error = fixture.routed.register_server(&server()).unwrap_err();

    assert!(registry::was_replaced(&error));
    assert!(format!("{error:#}").contains("tool catalog"));
    assert_eq!(fixture.local.registered_servers().unwrap(), [server()]);
    assert!(!has_resident(&fixture));
    assert_eq!(fixture.reloads.load(Ordering::SeqCst), 1);
}

#[test]
fn reconciliation_failure_preserves_the_original_durability_error() {
    let fixture = fixture("", failing_directory_sync);
    std::fs::create_dir(fixture.home.join("tool-catalog.sqlite")).unwrap();

    let error = fixture.routed.register_server(&server()).unwrap_err();

    assert!(registry::was_replaced(&error));
    let detail = format!("{error:#}");
    assert!(
        detail.contains("injected registry directory sync failure"),
        "{detail}"
    );
    assert!(
        detail.contains("durability after a crash is uncertain"),
        "{detail}"
    );
    assert!(detail.contains("tool catalog"), "{detail}");
    assert_eq!(fixture.local.registered_servers().unwrap(), [server()]);
    assert!(!has_resident(&fixture));
    assert_eq!(fixture.reloads.load(Ordering::SeqCst), 1);
}

fn assert_gateway_reload_failure(
    sync: fn(&Path) -> io::Result<()>,
    expected_primary: Option<&str>,
) {
    for remove in [false, true] {
        let document = if remove {
            "[mcp_servers.example]\nurl = 'https://example.test/mcp'\n"
        } else {
            ""
        };
        let fixture = fixture(document, sync);
        let generation = ToolCatalogCache::new(&fixture.home).generation().unwrap();
        fixture.gateway_failure.store(true, Ordering::SeqCst);

        let result = if remove {
            fixture.routed.unregister_server("example").map(|_| ())
        } else {
            fixture.routed.register_server(&server())
        };
        let error = result.unwrap_err();

        assert!(registry::was_replaced(&error));
        let detail = format!("{error:#}");
        assert!(
            detail.contains("injected Gateway reload failure"),
            "{detail}"
        );
        if let Some(primary) = expected_primary {
            assert!(detail.contains(primary), "{detail}");
            assert!(
                detail.contains("durability after a crash is uncertain"),
                "{detail}"
            );
        } else {
            assert!(
                detail.contains("registry was replaced, but follow-up failed"),
                "{detail}"
            );
        }
        assert_eq!(
            fixture.local.registered_servers().unwrap(),
            if remove { vec![] } else { vec![server()] }
        );
        assert_reconciled(&fixture, generation);
    }
}

#[test]
fn gateway_reload_failure_after_a_successful_write_reports_the_installed_registry() {
    assert_gateway_reload_failure(registry::sync_directory, None);
}

#[test]
fn gateway_reload_failure_preserves_the_original_post_replacement_error() {
    assert_gateway_reload_failure(
        failing_directory_sync,
        Some("injected registry directory sync failure"),
    );
}
