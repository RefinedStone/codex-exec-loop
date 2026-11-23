//! Validated storage use cases. The host owns activation, jobs, scopes and cancellation.
use crate::application::port::outbound::storage_analysis_port::StorageAnalysisPort;
use crate::domain::storage_analysis::*;
use anyhow::{Result, anyhow, ensure};
use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};

#[derive(Clone)]
pub struct StorageAnalysisService {
    port: Arc<dyn StorageAnalysisPort>,
}
impl StorageAnalysisService {
    pub fn live_children(&self, query: &StorageLiveQuery) -> Result<StorageLivePage> {
        validate_id(&query.job_id)?;
        self.port.live_children(query)
    }
    pub fn prioritize(&self, job_id: &str, node_id: &str) -> Result<StoragePriorityResult> {
        validate_id(job_id)?;
        self.port.prioritize(job_id, node_id)
    }
    pub fn new(port: Arc<dyn StorageAnalysisPort>) -> Self {
        Self { port }
    }
    pub fn list_volumes(&self) -> Result<Vec<StorageVolume>> {
        self.port.list_volumes()
    }
    pub fn scan(
        &self,
        request: &ScanRequest,
        cancelled: &AtomicBool,
        progress: &(dyn Fn(ScanProgress) + Sync),
    ) -> Result<StorageSnapshot> {
        request.options.validate().map_err(|e| anyhow!(e))?;
        validate_id(&request.job_id)?;
        self.port.scan(request, cancelled, progress)
    }
    pub fn snapshot(&self, id: &str) -> Result<StorageSnapshot> {
        validate_id(id)?;
        self.port.snapshot(id)
    }
    pub fn get_node(&self, snapshot: &str, node: &str, root: Option<&str>) -> Result<StorageNode> {
        validate_id(snapshot)?;
        self.port.get_node(snapshot, node, root)
    }
    pub fn list_children(&self, query: &StorageQuery) -> Result<StoragePage<StorageNode>> {
        validate_query(query)?;
        self.port.list_children(query)
    }
    pub fn list_files(&self, query: &StorageQuery) -> Result<StoragePage<StorageNode>> {
        validate_query(query)?;
        self.port.list_files(query)
    }
    pub fn find_hotspots(
        &self,
        query: &StorageQuery,
        thresholds: &HotspotThresholds,
    ) -> Result<StoragePage<StorageHotspot>> {
        validate_query(query)?;
        ensure!(
            matches!(query.sort, StorageSort::AllocatedDesc)
                && query.name_filter.is_none()
                && query.extension_filter.is_none()
                && !query.recursive,
            "storage_hotspot_options_unsupported"
        );
        thresholds.values().map_err(|e| anyhow!(e))?;
        self.port.find_hotspots(query, thresholds)
    }
    pub fn ancestors(
        &self,
        snapshot: &str,
        node: &str,
        root: Option<&str>,
    ) -> Result<Vec<StorageNode>> {
        validate_id(snapshot)?;
        self.port.ancestors(snapshot, node, root)
    }
    pub fn is_descendant(&self, snapshot: &str, node: &str, root: &str) -> Result<bool> {
        validate_id(snapshot)?;
        self.port.is_descendant(snapshot, node, root)
    }
    pub fn reveal_path(&self, snapshot: &str, node: &str) -> Result<PathBuf> {
        validate_id(snapshot)?;
        self.port.reveal_path(snapshot, node)
    }
    pub fn cleanup(&self, snapshot: &str) -> Result<()> {
        validate_id(snapshot)?;
        self.port.cleanup(snapshot)
    }
}
fn validate_query(q: &StorageQuery) -> Result<()> {
    validate_id(&q.snapshot_id)?;
    q.validate().map_err(|e| anyhow!(e))
}
fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(anyhow!("storage_not_found"));
    }
    Ok(())
}
