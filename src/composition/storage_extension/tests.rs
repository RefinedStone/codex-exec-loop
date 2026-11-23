//! A real metadata fixture through the registered host, scanner, schemas and scoped share service.
use super::*;
use crate::application::port::outbound::extension_state_store_port::ExtensionStateStorePort;
use crate::application::service::kernel_extensions::{
    ExtensionService, registry::ExtensionRegistry,
};
use crate::domain::kernel_extension::{ExtensionDurableState, ExtensionLifecycle};
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Default)]
struct MemoryStore(Mutex<ExtensionDurableState>);
impl ExtensionStateStorePort for MemoryStore {
    fn load(&self) -> Result<ExtensionDurableState> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&self, state: &ExtensionDurableState) -> Result<()> {
        *self.0.lock().unwrap() = state.clone();
        Ok(())
    }
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-storage-package-{:032x}",
            rand::random::<u128>()
        ));
        std::fs::create_dir_all(root.join("source/selected")).unwrap();
        std::fs::create_dir_all(root.join("source/hidden")).unwrap();
        std::fs::write(root.join("source/selected/data.bin"), [1; 8192]).unwrap();
        std::fs::write(root.join("source/hidden/private.bin"), [2; 4096]).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn wait_until(mut check: impl FnMut() -> bool) {
    let started = Instant::now();
    while !check() {
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "storage fixture deadline exceeded"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn storage_extension_real_scan_and_shared_read_use_one_registered_index() {
    let fixture = Fixture::new();
    let storage = StorageAnalysisService::new(Arc::new(NativeStorageAnalysisAdapter::fixture(
        fixture.0.join("indexes"),
        fixture.0.join("source"),
    )));
    let registry =
        ExtensionRegistry::build(vec![registration_with(storage)], ExtensionScope::LocalUser)
            .unwrap();
    let host = ExtensionService::new(
        registry,
        Arc::new(MemoryStore::default()),
        ExtensionScope::LocalUser,
        None,
        BTreeSet::from([
            "metadata.read".into(),
            "storage.read".into(),
            "git.read".into(),
        ]),
    )
    .unwrap();
    assert_eq!(host.catalog().len(), 1);
    assert!(host.catalog()[0].registration_errors.is_empty());
    let epoch = Arc::new(AtomicU64::new(1));
    let mut context = host.context("desktop-fixture".into(), None);
    context.verified_thread = Some("thread-fixture".into());
    context.conversation_generation = Some((1, epoch.clone()));
    let invoke = |context: &ExtensionContext, action: &str, input: Value, key: Option<&str>| {
        host.call(
            context.clone(),
            STORAGE_EXTENSION_ID,
            1,
            action,
            input,
            key.map(str::to_owned),
        )
    };
    assert!(invoke(&context, "volumes", json!({}), None).is_err());
    invoke(
        &context,
        "configure",
        json!({"enabled":true,"expected_revision":0}),
        None,
    )
    .unwrap();
    assert!(
        !fixture.0.join("indexes").exists(),
        "enable must not scan or allocate a job index"
    );
    let volumes = invoke(&context, "volumes", json!({}), None).unwrap();
    assert_eq!(volumes["volumes"][0]["id"], "fixture-volume");
    assert_eq!(
        invoke(&context, "jobs", json!({}), None).unwrap()["jobs"],
        json!([])
    );
    let input = json!({"volume_id":"fixture-volume","options":{"concurrency":2}});
    let job = invoke(&context, "scan", input.clone(), Some("explicit-scan")).unwrap();
    let id = job["id"].as_str().unwrap().to_owned();
    let mut terminal = Value::Null;
    wait_until(|| {
        terminal = invoke(&context, "job", json!({"job_id":id}), None).unwrap();
        matches!(
            terminal["state"].as_str(),
            Some("completed" | "failed" | "cancelled")
        )
    });
    assert_eq!(terminal["state"], "completed", "{terminal}");
    let snapshot = invoke(&context, "snapshot", json!({"snapshot_id":id}), None).unwrap();
    assert_eq!(snapshot, terminal["result"]);
    assert_eq!(snapshot["files_scanned"], 2);
    let retry = invoke(&context, "scan", input, Some("explicit-scan")).unwrap();
    assert_eq!(retry["id"], id);
    let children = invoke(
        &context,
        "list_children",
        json!({"snapshot_id":id,"node_id":"n1"}),
        None,
    )
    .unwrap();
    let selected = children["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["name"] == "selected")
        .unwrap()
        .clone();
    let node_id = selected["id"].as_str().unwrap();
    let mut unverified = context.clone();
    unverified.verified_thread = None;
    assert!(
        invoke(
            &unverified,
            "share_preview",
            json!({
        "snapshot_id":id,"node_id":node_id,"thread_id":"thread-fixture"}),
            None
        )
        .is_err()
    );
    let preview = invoke(
        &context,
        "share_preview",
        json!({
        "snapshot_id":id,"node_id":node_id,"thread_id":"thread-fixture"}),
        None,
    )
    .unwrap();
    let grant = invoke(
        &context,
        "share_create",
        json!({
        "preview_id":preview["id"],"thread_id":"thread-fixture"}),
        None,
    )
    .unwrap();
    assert!(!grant["prompt"].as_str().unwrap().contains("selected"));
    let shared = invoke(
        &context,
        "shared_get_node",
        json!({"grant_id":grant["id"],"node_id":node_id}),
        None,
    )
    .unwrap();
    assert_eq!(shared["node"]["id"], selected["id"]);
    assert_eq!(
        shared["node"]["allocated_bytes"],
        selected["allocated_bytes"]
    );
    assert!(shared["node"]["parent_id"].is_null());
    assert!(shared["node"]["ratio"]["denominator_bytes"].is_null());
    assert!(
        invoke(
            &context,
            "shared_get_node",
            json!({"grant_id":grant["id"],"node_id":"n1"}),
            None
        )
        .is_err()
    );
    let page = invoke(
        &context,
        "shared_list_children",
        json!({"grant_id":grant["id"],"node_id":node_id}),
        None,
    )
    .unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    invoke(
        &context,
        "shared_find_hotspots",
        json!({"grant_id":grant["id"],"node_id":node_id}),
        None,
    )
    .unwrap();
    let target = invoke(
        &context,
        "resolve_evidence",
        json!({
        "reference":shared["reference"],"thread_id":"thread-fixture"}),
        None,
    )
    .unwrap();
    assert_eq!(target["node_id"], node_id);
    epoch.fetch_add(1, Ordering::AcqRel);
    assert!(
        invoke(
            &context,
            "resolve_evidence",
            json!({
        "reference":shared["reference"],"thread_id":"thread-fixture"}),
            None
        )
        .is_err()
    );
    context.conversation_generation = Some((2, epoch.clone()));
    invoke(
        &context,
        "share_revoke",
        json!({"grant_id":grant["id"]}),
        None,
    )
    .unwrap();
    assert!(
        invoke(
            &context,
            "shared_get_node",
            json!({"grant_id":grant["id"],"node_id":node_id}),
            None
        )
        .is_err()
    );
    let revision = host.catalog()[0].revision;
    invoke(
        &context,
        "configure",
        json!({"enabled":false,"expected_revision":revision}),
        None,
    )
    .unwrap();
    wait_until(|| host.catalog()[0].lifecycle == ExtensionLifecycle::Disabled);
    assert!(!fixture.0.join("indexes").join(&id).exists());
    assert!(
        invoke(
            &context,
            "get_node",
            json!({"snapshot_id":id,"node_id":node_id}),
            None
        )
        .is_err()
    );
    assert!(invoke(&context, "job", json!({"job_id":id}), None).unwrap()["result"].is_null());
    host.shutdown();
}

mod desktop_bridge;
