//! Filesystem enumeration and the job-owned persistent index are one outbound boundary.
use crate::domain::storage_analysis::*;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

pub trait StorageAnalysisPort: Send + Sync {
    fn list_volumes(&self) -> Result<Vec<StorageVolume>>;
    fn scan(
        &self,
        request: &ScanRequest,
        cancelled: &AtomicBool,
        progress: &(dyn Fn(ScanProgress) + Sync),
    ) -> Result<StorageSnapshot>;
    fn snapshot(&self, snapshot_id: &str) -> Result<StorageSnapshot>;
    fn get_node(
        &self,
        snapshot_id: &str,
        node_id: &str,
        scope_root_id: Option<&str>,
    ) -> Result<StorageNode>;
    fn list_children(&self, query: &StorageQuery) -> Result<StoragePage<StorageNode>>;
    fn list_files(&self, query: &StorageQuery) -> Result<StoragePage<StorageNode>>;
    fn find_hotspots(
        &self,
        query: &StorageQuery,
        thresholds: &HotspotThresholds,
    ) -> Result<StoragePage<StorageHotspot>>;
    fn ancestors(
        &self,
        snapshot_id: &str,
        node_id: &str,
        scope_root_id: Option<&str>,
    ) -> Result<Vec<StorageNode>>;
    fn is_descendant(&self, snapshot_id: &str, node_id: &str, root_id: &str) -> Result<bool>;
    fn reveal_path(&self, snapshot_id: &str, node_id: &str) -> Result<PathBuf>;
    fn cleanup(&self, snapshot_id: &str) -> Result<()>;
}
