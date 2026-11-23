//! Native metadata scanner and private, per-job SQLite indexes.
#[cfg(test)]
mod evidence_tests;
mod index;
mod live;
mod native;
mod scanner;
mod worktree;

use crate::application::port::outbound::storage_analysis_port::StorageAnalysisPort;
use crate::domain::storage_analysis::*;
use anyhow::{Result, anyhow, bail};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock, Weak, atomic::AtomicBool},
};

pub type StorageRuntimeObserver = dyn Fn(&Path) -> Vec<StorageEvidence> + Send + Sync;
pub type StorageKnownRootsObserver = dyn Fn(usize) -> StorageRuntimeRoots + Send + Sync;

#[derive(Default)]
pub struct StorageRuntimeRoots {
    pub paths: Vec<PathBuf>,
    pub complete: bool,
}

#[derive(Clone)]
pub struct StorageRuntimeProbe {
    pub observe: Arc<StorageRuntimeObserver>,
    pub known_roots: Arc<StorageKnownRootsObserver>,
}

type ActiveScans = Mutex<HashMap<String, Arc<Mutex<live::ScanSchedulingState>>>>;
struct ScanRegistration<'a> {
    active: &'a ActiveScans,
    id: String,
}
impl Drop for ScanRegistration<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.id);
        }
    }
}

pub struct NativeStorageAnalysisAdapter {
    active: ActiveScans,
    live_readers: RwLock<()>,
    runtime_probe: Option<StorageRuntimeProbe>,
    root: PathBuf,
    leases: Mutex<HashMap<String, Weak<RwLock<()>>>>,
    // Conservative host budget: concurrent jobs never multiply device I/O concurrency.
    device_budget: RwLock<()>,
    #[cfg(test)]
    fixture_root: Option<PathBuf>,
}
impl NativeStorageAnalysisAdapter {
    pub fn new(index_root: PathBuf) -> Self {
        Self {
            root: index_root,
            runtime_probe: None,
            active: Mutex::new(HashMap::new()),
            live_readers: RwLock::new(()),
            leases: Mutex::new(HashMap::new()),
            device_budget: RwLock::new(()),
            #[cfg(test)]
            fixture_root: None,
        }
    }
    pub fn with_runtime_probe(mut self, probe: StorageRuntimeProbe) -> Self {
        self.runtime_probe = Some(probe);
        self
    }
    fn lease(&self, id: &str) -> Result<Arc<RwLock<()>>> {
        validate_id(id)?;
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| anyhow!("storage_lock_poisoned"))?;
        leases.retain(|_, lease| lease.strong_count() > 0);
        if let Some(lease) = leases.get(id).and_then(Weak::upgrade) {
            return Ok(lease);
        }
        let lease = Arc::new(RwLock::new(()));
        leases.insert(id.into(), Arc::downgrade(&lease));
        Ok(lease)
    }
    fn job_dir(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        Ok(self.root.join(id))
    }
    fn query<T>(
        &self,
        id: &str,
        read: impl FnOnce(&index::StorageIndex) -> Result<T>,
    ) -> Result<T> {
        // Query temporary files must not contribute to a simultaneous scan on this host.
        let _activity = self
            .device_budget
            .try_read()
            .map_err(|_| anyhow!("storage_scan_in_progress"))?;
        let lease = self.lease(id)?;
        let _guard = lease
            .try_read()
            .map_err(|_| anyhow!("storage_snapshot_pending"))?;
        let db = index::StorageIndex::open(&self.job_dir(id)?)?;
        read(&db)
    }
    #[cfg(test)]
    pub(crate) fn fixture(index_root: PathBuf, scan_root: PathBuf) -> Self {
        let mut adapter = Self::new(index_root);
        adapter.fixture_root = Some(scan_root);
        adapter
    }
}
impl StorageAnalysisPort for NativeStorageAnalysisAdapter {
    fn live_children(&self, q: &StorageLiveQuery) -> Result<StorageLivePage> {
        let _reader = self
            .live_readers
            .read()
            .map_err(|_| anyhow!("storage_live_unavailable"))?;
        live::children(&self.job_dir(&q.job_id)?, q)
    }
    fn prioritize(&self, job_id: &str, node_id: &str) -> Result<StoragePriorityResult> {
        let _reader = self
            .live_readers
            .read()
            .map_err(|_| anyhow!("storage_live_unavailable"))?;
        validate_id(job_id)?;
        let state = self
            .active
            .lock()
            .map_err(|_| anyhow!("storage_priority_unavailable"))?
            .get(job_id)
            .cloned()
            .ok_or_else(|| anyhow!("storage_job_not_running"))?;
        let pending = live::pending(&self.job_dir(job_id)?, node_id)?;
        let mut state = state
            .lock()
            .map_err(|_| anyhow!("storage_priority_unavailable"))?;
        let (queued, queue_length) = match pending {
            Some(id) => state.enqueue(id)?,
            None => (false, state.len()),
        };
        Ok(StoragePriorityResult {
            job_id: job_id.into(),
            node_id: node_id.into(),
            queued,
            queue_length,
        })
    }
    fn list_volumes(&self) -> Result<Vec<StorageVolume>> {
        #[cfg(test)]
        if let Some(path) = &self.fixture_root {
            return Ok(vec![native::fixture_volume(path)?]);
        }
        native::list_volumes()
    }
    fn scan(
        &self,
        request: &ScanRequest,
        cancelled: &AtomicBool,
        progress: &(dyn Fn(ScanProgress) + Sync),
    ) -> Result<StorageSnapshot> {
        request.options.validate().map_err(|e| anyhow!(e))?;
        let lease = self.lease(&request.job_id)?;
        let _guard = lease
            .try_write()
            .map_err(|_| anyhow!("storage_job_in_use"))?;
        let _device = self
            .device_budget
            .write()
            .map_err(|_| anyhow!("storage_lock_poisoned"))?;
        let volume = self
            .list_volumes()?
            .into_iter()
            .find(|v| v.id == request.volume_id && v.supported)
            .ok_or_else(|| anyhow!("storage_volume_not_found"))?;
        let root = PathBuf::from(&volume.root_path);
        let dir = self.job_dir(&request.job_id)?;
        if dir.exists() {
            return Err(anyhow!("storage_job_already_exists"));
        }
        std::fs::create_dir_all(&self.root)?;
        reject_symlink(&self.root)?;
        std::fs::create_dir(&dir)?;
        let mut db = index::StorageIndex::create(&dir, &request.job_id, &volume)?;
        let scheduling = Arc::new(Mutex::new(live::ScanSchedulingState::default()));
        self.active
            .lock()
            .map_err(|_| anyhow!("storage_priority_unavailable"))?
            .insert(request.job_id.clone(), scheduling.clone());
        let _registration = ScanRegistration {
            active: &self.active,
            id: request.job_id.clone(),
        };
        let result = scanner::scan(
            &mut db,
            &root,
            &self.root,
            request,
            scanner::ScanControl {
                cancelled,
                progress,
                runtime_probe: self.runtime_probe.as_ref(),
                scheduling: &scheduling,
            },
        );
        match result {
            Ok(()) => db.snapshot(),
            Err(error) => {
                db.finish(
                    StorageState::Failed,
                    Some(StorageIssue::one("scan_failed", error.to_string())),
                )?;
                Err(error)
            }
        }
    }
    fn snapshot(&self, id: &str) -> Result<StorageSnapshot> {
        self.query(id, |db| {
            let mut snapshot = db.snapshot()?;
            if snapshot.state == StorageState::Running {
                snapshot.state = StorageState::Interrupted;
                snapshot.issues.push(StorageIssue::one(
                    "host_interrupted",
                    "The previous scan did not finish; it is not automatically restarted",
                ));
            }
            Ok(snapshot)
        })
    }
    fn get_node(&self, s: &str, n: &str, r: Option<&str>) -> Result<StorageNode> {
        self.query(s, |db| db.get_node(n, r))
    }
    fn list_children(&self, q: &StorageQuery) -> Result<StoragePage<StorageNode>> {
        self.query(&q.snapshot_id, |db| db.page(q, false))
    }
    fn list_files(&self, q: &StorageQuery) -> Result<StoragePage<StorageNode>> {
        self.query(&q.snapshot_id, |db| db.page(q, true))
    }
    fn find_hotspots(
        &self,
        q: &StorageQuery,
        t: &HotspotThresholds,
    ) -> Result<StoragePage<StorageHotspot>> {
        self.query(&q.snapshot_id, |db| db.hotspots(q, t))
    }
    fn ancestors(&self, s: &str, n: &str, r: Option<&str>) -> Result<Vec<StorageNode>> {
        self.query(s, |db| db.ancestors(n, r))
    }
    fn is_descendant(&self, s: &str, n: &str, r: &str) -> Result<bool> {
        self.query(s, |db| db.is_descendant(n, r))
    }
    fn reveal_path(&self, s: &str, n: &str) -> Result<PathBuf> {
        self.query(s, |db| {
            let path = db.path(n)?;
            let root = db.path("n1")?;
            let actual = std::fs::canonicalize(&path)?;
            let root = std::fs::canonicalize(root)?;
            if !actual.starts_with(&root) {
                bail!("storage_path_changed");
            }
            let observed = native::metadata(&path, &root, None)?;
            if observed.issue.is_some()
                || observed.identity.is_none()
                || observed.identity != db.identity(n)?
            {
                bail!("storage_path_changed");
            }
            Ok(actual)
        })
    }
    fn cleanup(&self, id: &str) -> Result<()> {
        let lease = self.lease(id)?;
        let _guard = lease
            .write()
            .map_err(|_| anyhow!("storage_lock_poisoned"))?;
        let _live_readers = self
            .live_readers
            .write()
            .map_err(|_| anyhow!("storage_live_unavailable"))?;
        let dir = self.job_dir(id)?;
        if !dir.exists() {
            return Ok(());
        }
        reject_symlink(&self.root)?;
        reject_symlink(&dir)?;
        // Open and verify the owner before deleting only this job's private directory.
        {
            let db = index::StorageIndex::open(&dir)?;
            if db.snapshot()?.id != id {
                bail!("storage_owner_mismatch");
            }
        }
        validate_cleanup_entries(&dir)?;
        remove_validated_index_files(&dir)
    }
}
const INDEX_FILES: [&str; 4] = [
    "index.sqlite3",
    "index.sqlite3-journal",
    "index.sqlite3-wal",
    "index.sqlite3-shm",
];
fn validate_cleanup_entries(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let name = entry?.file_name();
        if !name
            .to_str()
            .is_some_and(|name| INDEX_FILES.contains(&name))
        {
            bail!("storage_unowned_index_entry");
        }
    }
    Ok(())
}
fn remove_validated_index_files(dir: &Path) -> Result<()> {
    // Never delete a second enumeration: a new, foreign entry must survive the race.
    for name in INDEX_FILES {
        match std::fs::remove_file(dir.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    std::fs::remove_dir(dir)?;
    Ok(())
}
fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        bail!("storage_not_found");
    }
    Ok(())
}
fn reject_symlink(path: &Path) -> Result<()> {
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        bail!("storage_unsafe_index_path");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if std::fs::symlink_metadata(path)?.file_attributes() & 0x400 != 0 {
            bail!("storage_unsafe_index_path");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod lifetime_tests;

#[cfg(test)]
mod benchmark;
