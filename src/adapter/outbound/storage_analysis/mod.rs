//! Native metadata scanner and private, per-job SQLite indexes.
mod index;
mod native;
mod scanner;
mod worktree;

use crate::application::port::outbound::storage_analysis_port::StorageAnalysisPort;
use crate::domain::storage_analysis::*;
use anyhow::{Result, anyhow, bail};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock, atomic::AtomicBool},
};

pub type StorageRuntimeProbe = Arc<dyn Fn(&Path) -> Vec<StorageEvidence> + Send + Sync>;

pub struct NativeStorageAnalysisAdapter {
    runtime_probe: Option<StorageRuntimeProbe>,
    root: PathBuf,
    leases: Mutex<HashMap<String, Arc<RwLock<()>>>>,
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
        Ok(self
            .leases
            .lock()
            .map_err(|_| anyhow!("storage_lock_poisoned"))?
            .entry(id.into())
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone())
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
        let result = scanner::scan(
            &mut db,
            &root,
            &self.root,
            request,
            cancelled,
            progress,
            self.runtime_probe.as_deref(),
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
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            if !matches!(
                name.to_str(),
                Some(
                    "index.sqlite3"
                        | "index.sqlite3-journal"
                        | "index.sqlite3-wal"
                        | "index.sqlite3-shm"
                )
            ) {
                bail!("storage_unowned_index_entry");
            }
        }
        for entry in std::fs::read_dir(&dir)? {
            std::fs::remove_file(entry?.path())?;
        }
        std::fs::remove_dir(dir)?;
        Ok(())
    }
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
