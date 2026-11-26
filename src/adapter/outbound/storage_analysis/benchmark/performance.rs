//! Opt-in native performance worker. A run reads only a fixture created by prepare.
use super::super::{NativeStorageAnalysisAdapter, live::ScanSchedulingState};
use crate::application::port::outbound::storage_analysis_port::StorageAnalysisPort;
use crate::domain::storage_analysis::{
    HotspotThresholds, ScanOptions, ScanRequest, StorageQuery, StorageSort, StorageState,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const MARKER: &str = "akra-storage-performance-fixture.json";
const FIXTURE_KIND: &str = "akra_storage_performance_v1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    mode: String,
    fixture_root: PathBuf,
    #[serde(default, rename = "case")]
    case_name: Option<String>,
    #[serde(default = "sequential")]
    concurrency: usize,
    run_id: String,
    #[serde(default)]
    cancel_after_first_file: bool,
}
fn sequential() -> usize {
    1
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Case {
    name: String,
    files_per_directory: Vec<usize>,
    bytes_per_file: usize,
}
impl Case {
    fn files(&self) -> u64 {
        self.files_per_directory.iter().sum::<usize>() as u64
    }
    fn bytes(&self) -> u64 {
        self.files() * self.bytes_per_file as u64
    }
    fn directories(&self) -> u64 {
        self.files_per_directory.len() as u64 + 1
    }
}
#[derive(Deserialize, Serialize)]
struct Manifest {
    kind: String,
    cases: Vec<Case>,
}
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "small_many".into(),
            files_per_directory: vec![512; 16],
            bytes_per_file: 1024,
        },
        Case {
            name: "large_few".into(),
            files_per_directory: vec![8, 1, 1, 1, 1],
            bytes_per_file: 32 * 1024 * 1024,
        },
    ]
}

fn emit(request: &Request, event: &str, fields: Value) -> Result<()> {
    let mut value = json!({
        "kind": "storage_performance",
        "event": event,
        "run_id": request.run_id,
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
            .context("set AKRA_STORAGE_PERFORMANCE_REQUEST to the explicit fixture request")?,
    )?;
    ensure!(
        request.fixture_root.is_absolute()
            && request.fixture_root.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("akra-storage-performance-")
            }),
        "the fixture must be a newly named absolute akra-storage-performance-* directory"
    );
    super::super::validate_id(&request.run_id)?;
    ScanOptions {
        concurrency: request.concurrency,
    }
    .validate()
    .map_err(anyhow::Error::msg)?;
    match request.mode.as_str() {
        "prepare" => prepare(&request),
        "run" => measure(&request),
        _ => bail!("mode must be prepare or run"),
    }
}

fn prepare(request: &Request) -> Result<()> {
    // create_dir, never create_dir_all: an existing directory is not owned by this worker.
    fs::create_dir(&request.fixture_root).context("fixture root must not already exist")?;
    let fixture_cases = cases();
    let block = vec![0x5a_u8; 1024 * 1024];
    for case in &fixture_cases {
        let case_root = request.fixture_root.join(&case.name);
        fs::create_dir(&case_root)?;
        for (directory, count) in case.files_per_directory.iter().enumerate() {
            let folder = case_root.join(format!("d{directory:02}"));
            fs::create_dir(&folder)?;
            for file in 0..*count {
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(folder.join(format!("f{file:04}.bin")))?;
                let mut remaining = case.bytes_per_file;
                while remaining > 0 {
                    let count = remaining.min(block.len());
                    output.write_all(&block[..count])?;
                    remaining -= count;
                }
            }
        }
    }
    let manifest = Manifest {
        kind: FIXTURE_KIND.into(),
        cases: fixture_cases,
    };
    let mut marker = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(request.fixture_root.join(MARKER))?;
    serde_json::to_writer_pretty(&mut marker, &manifest)?;
    marker.flush()?;
    emit(
        request,
        "prepared",
        json!({
            "cases": manifest.cases,
            "logical_bytes": manifest.cases.iter().map(Case::bytes).sum::<u64>(),
            "cold_cache_controlled": false,
            "preparation": "ordinary create/write/close; no cache flush or cache eviction",
        }),
    )
}

fn load_case(request: &Request) -> Result<Case> {
    super::super::reject_symlink(&request.fixture_root)?;
    let marker = request.fixture_root.join(MARKER);
    super::super::reject_symlink(&marker)?;
    let manifest: Manifest = serde_json::from_slice(&fs::read(marker)?)?;
    ensure!(
        manifest.kind == FIXTURE_KIND && manifest.cases == cases(),
        "fixture manifest does not match the fixed synthetic datasets"
    );
    let name = request.case_name.as_deref().context("run requires case")?;
    manifest
        .cases
        .into_iter()
        .find(|case| case.name == name)
        .context("unknown case")
}

#[derive(Default)]
struct Observations {
    first_progress_us: Option<u128>,
    first_file_us: Option<u128>,
    cancel_at: Option<Instant>,
    cancel_files: Option<u64>,
    scheduling: Option<Arc<Mutex<ScanSchedulingState>>>,
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
    let case = load_case(request)?;
    let scan_root = request.fixture_root.join(&case.name);
    super::super::reject_symlink(&scan_root)?;
    let index_root = request.fixture_root.join("indexes");
    let adapter = NativeStorageAnalysisAdapter::fixture(index_root.clone(), scan_root);
    let cancelled = AtomicBool::new(false);
    let observations = Mutex::new(Observations::default());
    let memory_begin = memory()?;
    let started = Instant::now();
    emit(
        request,
        "begin",
        json!({
            "case": case.name, "concurrency": request.concurrency, "elapsed_us": 0,
            "volume_id": "fixture-volume",
            "cancel_after_first_file": request.cancel_after_first_file,
        }),
    )?;
    let snapshot = adapter.scan(
        &ScanRequest {
            job_id: request.run_id.clone(),
            volume_id: "fixture-volume".into(),
            options: ScanOptions {
                concurrency: request.concurrency,
            },
        },
        &cancelled,
        &|progress| {
            let mut observations = observations.lock().unwrap();
            observations
                .first_progress_us
                .get_or_insert_with(|| started.elapsed().as_micros());
            if observations.scheduling.is_none() {
                observations.scheduling =
                    adapter.active.lock().unwrap().get(&request.run_id).cloned();
            }
            if progress.files_scanned > 0 {
                observations
                    .first_file_us
                    .get_or_insert_with(|| started.elapsed().as_micros());
                if request.cancel_after_first_file && observations.cancel_at.is_none() {
                    observations.cancel_at = Some(Instant::now());
                    observations.cancel_files = Some(progress.files_scanned);
                    cancelled.store(true, Ordering::Release);
                }
            }
            emit(
                request,
                "progress",
                json!({
                    "elapsed_us": started.elapsed().as_micros(), "progress": progress,
                }),
            )
            .expect("performance event output");
        },
    )?;
    let elapsed_us = started.elapsed().as_micros();
    let scan_finished_unix_us = SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros();
    let memory_after_scan = memory()?;
    let observations = observations.into_inner().unwrap();
    let scheduling = observations
        .scheduling
        .context("native scan scheduling trace")?;
    let trace = scheduling.lock().unwrap().measurement;
    let cancellation = if request.cancel_after_first_file {
        ensure!(
            snapshot.state == StorageState::Cancelled,
            "cancel did not end as cancelled"
        );
        let cancel_at = observations
            .cancel_at
            .context("cancel trigger was not reached")?;
        let last_dispatch = trace
            .last_dispatch
            .context("at least one directory was dispatched")?;
        let relative = if last_dispatch >= cancel_at {
            last_dispatch.duration_since(cancel_at).as_micros() as i128
        } else {
            -(cancel_at.duration_since(last_dispatch).as_micros() as i128)
        };
        Some(json!({
            "trigger": "first native file progress callback",
            "cancel_requested_us": cancel_at.duration_since(started).as_micros(),
            "last_dispatch_relative_to_cancel_us": relative,
            "new_dispatch_stop_us": relative.max(0),
            "dispatches_after_cancel": trace.dispatches_after_cancel,
            "dispatches": trace.dispatches,
            "cancellation_observed_us": trace.cancel_observed.map(|at| at.saturating_duration_since(cancel_at).as_micros()),
            "cancel_to_terminal_us": elapsed_us.saturating_sub(cancel_at.duration_since(started).as_micros()),
            "observed_when_cancelled": observations.cancel_files,
            "observed_at_terminal": snapshot.files_scanned,
            "dispatch_stop_separately_instrumented": true,
        }))
    } else {
        ensure!(
            snapshot.state == StorageState::Complete,
            "scan was not complete"
        );
        ensure!(snapshot.issues.is_empty(), "scan reported issues");
        ensure!(snapshot.files_scanned == case.files(), "file count changed");
        ensure!(
            snapshot.directories_scanned == case.directories(),
            "directory count changed"
        );
        None
    };
    let root_node = adapter.get_node(&request.run_id, "n1", None)?;
    if !request.cancel_after_first_file {
        ensure!(
            root_node.logical_bytes == case.bytes().to_string(),
            "logical bytes changed"
        );
    }
    let read_started = Instant::now();
    let children = adapter.list_children(&query(&request.run_id))?;
    let children_us = read_started.elapsed().as_micros();
    let thresholds = [
        ("default", HotspotThresholds::default()),
        (
            "more_rows",
            HotspotThresholds {
                absolute_bytes: (128_u64 * 1024 * 1024).to_string(),
                relative_min_bytes: (32_u64 * 1024 * 1024).to_string(),
                parent_percent: 5,
            },
        ),
        (
            "fewer_rows",
            HotspotThresholds {
                absolute_bytes: (2_u64 * 1024 * 1024 * 1024).to_string(),
                relative_min_bytes: (512_u64 * 1024 * 1024).to_string(),
                parent_percent: 25,
            },
        ),
    ];
    let mut comparisons = Vec::new();
    let mut default_hotspots = None;
    for (name, thresholds) in thresholds {
        let before = Instant::now();
        let hotspots = adapter.find_hotspots(&query(&request.run_id), &thresholds)?;
        comparisons.push(json!({
            "name": name, "thresholds": thresholds,
            "elapsed_us": before.elapsed().as_micros(), "returned_rows": hotspots.items.len(),
            "total_count": hotspots.total_count, "has_next_page": hotspots.next_cursor.is_some(),
            "included_bytes": hotspots.items.iter().map(|row| row.included_bytes.clone()).collect::<Vec<_>>(),
        }));
        if name == "default" {
            default_hotspots = Some(hotspots);
        }
    }
    let memory_final = memory()?;
    emit(
        request,
        "complete",
        json!({
            "elapsed_us": started.elapsed().as_micros(),
            "result": {
                "snapshot": snapshot, "root_node": root_node, "children": children,
                "hotspots": default_hotspots,
                "metrics": {
                    "case": case.name, "concurrency": request.concurrency,
                    "elapsed_us": elapsed_us,
                    "scan_finished_unix_us": scan_finished_unix_us,
                    "first_progress_us": observations.first_progress_us,
                    "first_file_observation_us": observations.first_file_us,
                    "expected_files": case.files(), "expected_directories": case.directories(),
                    "expected_logical_bytes": case.bytes().to_string(),
                    "children_page_us": children_us,
                    "index_bytes": fs::metadata(index_root.join(&request.run_id).join("index.sqlite3"))?.len(),
                    "process_working_set_begin_bytes": memory_begin.map(|memory| memory.0),
                    "process_peak_working_set_bytes": memory_after_scan.map(|memory| memory.1),
                    "process_peak_with_queries_bytes": memory_final.map(|memory| memory.1),
                    "memory_scope": "fresh native libtest worker, runtime and SQLite; fixture preparation, Gateway and Electron excluded",
                    "cold_cache_controlled": false,
                    "dispatches": trace.dispatches,
                    "threshold_comparisons": comparisons,
                    "cancellation": cancellation,
                }
            }
        }),
    )
}

#[cfg(windows)]
fn memory() -> Result<Option<(u64, u64)>> {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // PROCESS_MEMORY_COUNTERS is the documented fixed native prefix. Keeping this
    // test-only binding avoids broadening production Windows API features.
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged_pool: usize,
        quota_paged_pool: usize,
        quota_peak_nonpaged_pool: usize,
        quota_nonpaged_pool: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn K32GetProcessMemoryInfo(process: HANDLE, counters: *mut Counters, size: u32) -> i32;
    }
    let mut counters = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        ..Default::default()
    };
    // SAFETY: the pseudo handle is valid for this process and counters has the exact
    // PROCESS_MEMORY_COUNTERS layout with its full size supplied to the OS.
    let size = counters.cb;
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
    ensure!(
        ok != 0,
        "native process memory query failed: {}",
        io::Error::last_os_error()
    );
    Ok(Some((
        counters.working_set as u64,
        counters.peak_working_set as u64,
    )))
}

#[cfg(not(windows))]
fn memory() -> Result<Option<(u64, u64)>> {
    Ok(None)
}
