//! Separate nested native fixture for threshold choices; leaves the matrix worker unchanged.
use super::super::NativeStorageAnalysisAdapter;
use crate::application::port::outbound::storage_analysis_port::StorageAnalysisPort;
use crate::domain::storage_analysis::{
    HotspotThresholds, ScanOptions, ScanRequest, StorageQuery, StorageSort, StorageState,
};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    sync::{Mutex, atomic::AtomicBool},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const FILE_BYTES: u64 = 32 * 1024 * 1024;
const TOTAL_BYTES: u64 = 12 * FILE_BYTES;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    mode: String,
    fixture_root: PathBuf,
    case: String,
    #[serde(default = "one")]
    concurrency: usize,
    run_id: String,
    #[serde(default)]
    cancel_after_first_file: bool,
}
fn one() -> usize {
    1
}

fn emit(request: &Request, event: &str, fields: Value) -> Result<()> {
    let mut value = json!({
        "kind": "storage_performance", "event": event, "run_id": request.run_id,
        "event_unix_us": SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros(),
    });
    value
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    let mut output = io::stdout().lock();
    writeln!(output, "{value}")?;
    output.flush()?;
    Ok(())
}

pub(super) fn run() -> Result<()> {
    let request: Request = serde_json::from_str(
        &std::env::var("AKRA_STORAGE_PERFORMANCE_REQUEST")
            .context("explicit threshold fixture request")?,
    )?;
    ensure!(
        request.case == "threshold_tree"
            && request.concurrency == 1
            && !request.cancel_after_first_file,
        "threshold worker requires threshold_tree, concurrency 1, no cancellation"
    );
    ensure!(
        request.fixture_root.is_absolute()
            && request.fixture_root.file_name().is_some_and(|name| name
                .to_string_lossy()
                .starts_with("akra-storage-performance-")),
        "explicit absolute owned fixture path required"
    );
    super::super::validate_id(&request.run_id)?;
    super::super::reject_symlink(&request.fixture_root)?;
    let marker = request
        .fixture_root
        .join("akra-storage-performance-fixture.json");
    super::super::reject_symlink(&marker)?;
    let manifest: Value = serde_json::from_slice(&fs::read(marker)?)?;
    ensure!(
        manifest
            == json!({
                "kind":"akra_storage_performance_v1",
                "cases":[
                    {"name":"small_many","files_per_directory":vec![512;16],"bytes_per_file":1024},
                    {"name":"large_few","files_per_directory":[8,1,1,1,1],"bytes_per_file":FILE_BYTES}
                ]
            }),
        "source is not the fixed worker-owned fixture"
    );
    match request.mode.as_str() {
        "prepare-threshold" => prepare(&request),
        "run" => measure(&request),
        _ => anyhow::bail!("threshold mode must be prepare-threshold or run"),
    }
}

fn prepare(request: &Request) -> Result<()> {
    let root = request.fixture_root.join("threshold_tree");
    fs::create_dir(&root).context("nested fixture must not already exist")?;
    for directory in ["giant", "giant/child_big", "giant/child_small", "moderate"] {
        fs::create_dir(root.join(directory))?;
    }
    let source = request.fixture_root.join("large_few");
    super::super::reject_symlink(&source)?;
    let mut links = Vec::new();
    for file in 0..8 {
        links.push((
            format!("d00/f{file:04}.bin"),
            format!("giant/child_big/f{file:04}.bin"),
        ));
    }
    for (from, to) in [
        ("d01/f0000.bin", "giant/direct.bin"),
        ("d02/f0000.bin", "giant/child_small/small.bin"),
        ("d03/f0000.bin", "moderate/first.bin"),
        ("d04/f0000.bin", "moderate/second.bin"),
    ] {
        links.push((from.into(), to.into()));
    }
    for (from, to) in links {
        let from = source.join(from);
        super::super::reject_symlink(from.parent().unwrap())?;
        super::super::reject_symlink(&from)?;
        let metadata = fs::metadata(&from)?;
        ensure!(
            metadata.is_file() && metadata.len() == FILE_BYTES,
            "source file changed"
        );
        fs::hard_link(&from, root.join(to))?;
    }
    emit(
        request,
        "prepared",
        json!({
            "case":"threshold_tree", "files":12, "directories":5, "logical_bytes":TOTAL_BYTES,
            "cold_cache_controlled":false,
            "preparation":"hardlinks to the worker-owned large_few fixture; each file has one observed path in the separate nested scan root; no body rewrite",
            "physical_storage_shared_with":"large_few",
        }),
    )
}

fn query(id: &str) -> StorageQuery {
    StorageQuery {
        snapshot_id: id.into(),
        node_id: "n1".into(),
        limit: 100,
        cursor: None,
        sort: StorageSort::AllocatedDesc,
        name_filter: None,
        extension_filter: None,
        recursive: false,
        scope_root_id: None,
    }
}

fn measure(request: &Request) -> Result<()> {
    let root = request.fixture_root.join("threshold_tree");
    super::super::reject_symlink(&root)?;
    let adapter =
        NativeStorageAnalysisAdapter::fixture(request.fixture_root.join("threshold_indexes"), root);
    let cancelled = AtomicBool::new(false);
    let observations = Mutex::new((None::<u128>, None::<u128>));
    let started = Instant::now();
    emit(
        request,
        "begin",
        json!({
            "case":"threshold_tree", "concurrency":1, "volume_id":"fixture-volume",
            "elapsed_us":0, "cancel_after_first_file":false,
        }),
    )?;
    let snapshot = adapter.scan(
        &ScanRequest {
            job_id: request.run_id.clone(),
            volume_id: "fixture-volume".into(),
            options: ScanOptions { concurrency: 1 },
        },
        &cancelled,
        &|progress| {
            let mut observations = observations.lock().unwrap();
            observations
                .0
                .get_or_insert_with(|| started.elapsed().as_micros());
            if progress.files_scanned > 0 {
                observations
                    .1
                    .get_or_insert_with(|| started.elapsed().as_micros());
            }
            emit(
                request,
                "progress",
                json!({
                    "elapsed_us":started.elapsed().as_micros(), "progress":progress,
                }),
            )
            .expect("threshold event output");
        },
    )?;
    let elapsed_us = started.elapsed().as_micros();
    let finished = SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros();
    ensure!(
        snapshot.state == StorageState::Complete
            && snapshot.issues.is_empty()
            && snapshot.files_scanned == 12
            && snapshot.directories_scanned == 5,
        "nested native scan quality changed"
    );
    let root_node = adapter.get_node(&request.run_id, "n1", None)?;
    ensure!(
        root_node.logical_bytes == TOTAL_BYTES.to_string()
            && root_node.allocated_bytes.as_deref() == Some(TOTAL_BYTES.to_string().as_str()),
        "nested native byte observation changed"
    );
    let before = Instant::now();
    let children = adapter.list_children(&query(&request.run_id))?;
    let children_us = before.elapsed().as_micros();
    let candidates = [
        ("default", 3, HotspotThresholds::default()),
        (
            "more_rows",
            4,
            HotspotThresholds {
                absolute_bytes: (128_u64 * 1024 * 1024).to_string(),
                relative_min_bytes: FILE_BYTES.to_string(),
                parent_percent: 5,
            },
        ),
        (
            "fewer_rows",
            2,
            HotspotThresholds {
                absolute_bytes: (2_u64 * 1024 * 1024 * 1024).to_string(),
                relative_min_bytes: (512_u64 * 1024 * 1024).to_string(),
                parent_percent: 25,
            },
        ),
    ];
    let mut comparisons = Vec::new();
    let mut default_hotspots = None;
    for (name, count, thresholds) in candidates {
        let before = Instant::now();
        let hotspots = adapter.find_hotspots(&query(&request.run_id), &thresholds)?;
        let query_us = before.elapsed().as_micros();
        ensure!(
            hotspots.items.len() == count && hotspots.next_cursor.is_none(),
            "nested threshold frontier changed"
        );
        let included = hotspots.items.iter().try_fold(0_u64, |sum, row| {
            Ok::<_, anyhow::Error>(sum + row.included_bytes.parse::<u64>()?)
        })?;
        ensure!(
            included == TOTAL_BYTES,
            "threshold rows do not partition observed bytes"
        );
        comparisons.push(json!({
            "name":name, "thresholds":thresholds, "elapsed_us":query_us,
            "returned_rows":hotspots.items.len(), "total_count":hotspots.total_count,
            "has_next_page":false,
            "included_bytes":hotspots.items.iter().map(|row| row.included_bytes.clone()).collect::<Vec<_>>(),
            "rows":hotspots.items.iter().map(|row| json!({
                "name":row.node.name, "role":row.role, "included_bytes":row.included_bytes,
            })).collect::<Vec<_>>(),
        }));
        if name == "default" {
            default_hotspots = Some(hotspots);
        }
    }
    let observations = observations.into_inner().unwrap();
    emit(
        request,
        "complete",
        json!({
            "elapsed_us":started.elapsed().as_micros(),
            "result":{
                "snapshot":snapshot, "root_node":root_node, "children":children, "hotspots":default_hotspots,
                "metrics":{
                    "case":"threshold_tree", "concurrency":1, "elapsed_us":elapsed_us,
                    "scan_finished_unix_us":finished,
                    "first_progress_us":observations.0, "first_file_observation_us":observations.1,
                    "children_page_us":children_us, "process_peak_working_set_bytes":null,
                    "memory_scope":"not measured by this separate threshold worker",
                    "threshold_comparisons":comparisons, "cold_cache_controlled":false, "cancellation":null,
                }
            }
        }),
    )
}
