//! Manual Electron/CLI bridge: both adapters read one real SQLite index through one registered host.
//! Only these private test rows contain synthetic 12/9/2/1 GiB metadata. No large files are created.
use super::*;
use crate::application::port::outbound::storage_analysis_port::StorageAnalysisPort;
use crate::domain::kernel_extension::{
    ExtensionActivation, ExtensionCleanupState, ExtensionJobState,
};
use std::io::{BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};

const PREFIX: &str = "AKRA_STORAGE_FIXTURE ";
const THREAD: &str = "01a1abfb-8f0a-7581-a8e4-c2953347f470";
const JOB: &str = "desktop-sqlite-fixture";

struct DesktopFixture(PathBuf);
impl Drop for DesktopFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture() -> Result<(
    DesktopFixture,
    NativeStorageAnalysisAdapter,
    StorageSnapshot,
)> {
    let fixture = DesktopFixture(crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-storage-desktop-{:032x}",
        rand::random::<u128>()
    )));
    let source = fixture.0.join("source");
    for folder in ["target", "assets"] {
        std::fs::create_dir_all(source.join(folder))?;
        std::fs::write(source.join(folder).join("data.bin"), [])?;
    }
    std::fs::write(source.join("root.bin"), [])?;
    let indexes = fixture.0.join("indexes");
    let adapter = NativeStorageAnalysisAdapter::fixture(indexes.clone(), source);
    let snapshot = adapter.scan(
        &ScanRequest {
            job_id: JOB.into(),
            volume_id: "fixture-volume".into(),
            options: ScanOptions { concurrency: 1 },
        },
        &std::sync::atomic::AtomicBool::new(false),
        &|_| {},
    )?;
    assert_eq!(snapshot.state, StorageState::Complete);
    assert_eq!(
        (snapshot.files_scanned, snapshot.directories_scanned),
        (3, 3)
    );
    // Seed metadata after the tiny native enumeration, before any host or reader is exposed.
    // Use the real schema and read service; never synthesize a second set of response objects.
    let connection = rusqlite::Connection::open(indexes.join(JOB).join("index.sqlite3"))?;
    let rows = {
        let mut statement = connection.prepare(
            "SELECT n.id,n.kind,n.name,p.name,n.object_key FROM nodes n LEFT JOIN nodes p ON p.id=n.parent ORDER BY n.id"
        )?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (id, kind, name, parent, object_key) in rows {
        let units = if id == 1 {
            12
        } else {
            match (kind.as_str(), name.as_str(), parent.as_deref()) {
                ("directory", "target", _) | (_, _, Some("target")) => 9,
                ("directory", "assets", _) | (_, _, Some("assets")) => 2,
                ("file" | "direct_files", _, _) => 1,
                _ => bail!("unexpected fixture node"),
            }
        };
        let amount = format!("{:020}", units * 1024_u64.pow(3));
        connection.execute(
            "UPDATE nodes SET known=?1,logical=?1 WHERE id=?2",
            rusqlite::params![amount, id],
        )?;
        if kind == "file" {
            connection.execute(
                "UPDATE objects SET allocated=?1,logical=?1 WHERE key=?2",
                rusqlite::params![amount, object_key],
            )?;
        }
    }
    drop(connection);
    assert_eq!(adapter.snapshot(JOB)?, snapshot);
    assert_eq!(
        adapter
            .get_node(JOB, "n1", None)?
            .allocated_bytes
            .as_deref(),
        Some("12884901888")
    );
    Ok((fixture, adapter, snapshot))
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn host(
    adapter: NativeStorageAnalysisAdapter,
    snapshot: &StorageSnapshot,
) -> Result<Arc<ExtensionService>> {
    let mut state = ExtensionDurableState {
        host_id: "storage-desktop-fixture-host".into(),
        ..Default::default()
    };
    state.activations.insert(
        STORAGE_EXTENSION_ID.into(),
        ExtensionActivation {
            desired_enabled: true,
            lifecycle: ExtensionLifecycle::Enabled,
            revision: 1,
            error: None,
        },
    );
    let now = now_ms();
    let progress = ScanProgress {
        volume_id: snapshot.volume_id.clone(),
        snapshot_id: snapshot.id.clone(),
        files_scanned: snapshot.files_scanned,
        directories_scanned: snapshot.directories_scanned,
        known_allocated_bytes: "12884901888".into(),
        state: snapshot.state.clone(),
    };
    state.jobs.insert(
        JOB.into(),
        ExtensionJob {
            id: JOB.into(),
            extension: STORAGE_EXTENSION_ID.into(),
            action: "scan".into(),
            state: ExtensionJobState::Completed,
            generation: "fixture-seeded".into(),
            revision: 1,
            sequence: 1,
            progress: serde_json::to_value(progress)?,
            result: Some(serde_json::to_value(snapshot)?),
            error: None,
            cleanup: ExtensionCleanupState::Pending,
            created_at_ms: now,
            updated_at_ms: now,
            result_expires_at_ms: now + 24 * 60 * 60 * 1000,
            dedupe_key: None,
        },
    );
    ExtensionService::new(
        ExtensionRegistry::build(
            vec![registration_with(StorageAnalysisService::new(Arc::new(
                adapter,
            )))],
            ExtensionScope::LocalUser,
        )?,
        Arc::new(MemoryStore(Mutex::new(state))),
        ExtensionScope::LocalUser,
        None,
        BTreeSet::from([
            "metadata.read".into(),
            "storage.read".into(),
            "git.read".into(),
        ]),
    )
}
fn dispatch(
    host: &Arc<ExtensionService>,
    epoch: &Arc<AtomicU64>,
    request: &Value,
) -> Result<Value> {
    let params = &request["params"];
    match request["method"].as_str().context("missing method")? {
        "akra/kernel/extensions/attach" => Ok(json!({
            "kernel":host.host_id(), "scope":"local_user", "generation":host.generation()
        })),
        "akra/kernel/extensions/detach" => Ok(json!({})),
        "akra/kernel/extensions/list" => Ok(serde_json::to_value(host.catalog())?),
        "akra/kernel/extensions/call" => {
            anyhow::ensure!(params["kernel"] == host.host_id(), "wrong fixture host");
            let caller = request["caller"].as_str().unwrap_or("desktop");
            let mut context = host.context(format!("fixture-{caller}"), None);
            // Authentication is a separate Gateway test. This bridge fixes one verified test
            // conversation while the real CLI adapter still performs initialize/resume.
            context.verified_thread = Some(THREAD.into());
            context.conversation_generation = Some((1, epoch.clone()));
            host.call(
                context,
                params["extension"].as_str().context("missing extension")?,
                u32::try_from(params["version"].as_u64().context("missing version")?)?,
                params["action"].as_str().context("missing action")?,
                params["input"].clone(),
                params["idempotency_key"].as_str().map(str::to_owned),
            )
        }
        _ => bail!("unsupported fixture method"),
    }
}
fn emit(value: Value) -> Result<()> {
    println!("{PREFIX}{value}");
    std::io::stdout().flush()?;
    Ok(())
}
#[test]
#[ignore = "manual bridge for desktop/test/e2e/storage-service-roundtrip.mjs"]
fn storage_desktop_fixture_bridge() -> Result<()> {
    anyhow::ensure!(
        std::env::var("AKRA_STORAGE_FIXTURE_SERVER").ok().as_deref() == Some("1"),
        "launch via the Desktop fixture runner"
    );
    let (_fixture, adapter, snapshot) = fixture()?;
    let host = host(adapter, &snapshot)?;
    let epoch = Arc::new(AtomicU64::new(1));
    emit(
        json!({"ready":true,"kernel":host.host_id(),"generation":host.generation(),
        "thread_id":THREAD,"snapshot":snapshot,"metadata_fixture":true}),
    )?;
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        if request["method"] == "fixture/shutdown" {
            break;
        }
        let answer = match dispatch(&host, &epoch, &request) {
            Ok(result) => json!({"id":request["id"],"result":result}),
            Err(error) => {
                json!({"id":request["id"],"error":{"code":-32000,"message":format!("{error:#}")}})
            }
        };
        emit(answer)?;
    }
    host.shutdown();
    drop(host);
    Ok(())
}
