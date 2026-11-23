use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::storage_analysis::{HotspotThresholds, StorageNode, StorageSort};

/// Created from the server's successful upstream thread binding, never deserialized from RPC.
#[derive(Clone)]
pub(crate) struct StorageShareCaller {
    pub host_id: String,
    pub activation_generation: u64,
    pub connection_id: String,
    pub thread_id: String,
    pub conversation_epoch: u64,
    pub current_epoch: Arc<AtomicU64>,
}

impl StorageShareCaller {
    pub fn check(&self) -> Result<()> {
        if self.thread_id.is_empty()
            || self.conversation_epoch != self.current_epoch.load(Ordering::Acquire)
        {
            bail!("storage_share_unavailable");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SharePreviewRequest {
    pub snapshot_id: String,
    pub node_id: String,
    pub thread_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct StorageSharePreview {
    pub id: String,
    pub snapshot_id: String,
    pub revision: u64,
    pub node_id: String,
    pub thread_id: String,
    pub root: StorageNode,
    pub shared_fields: Vec<String>,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShareCreateRequest {
    pub preview_id: String,
    pub thread_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct StorageShareGrant {
    pub id: String,
    pub host_id: String,
    pub thread_id: String,
    pub snapshot_id: String,
    pub revision: u64,
    pub root_node_id: String,
    pub expires_at_ms: i64,
    pub reference: String,
    pub prompt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShareRevokeRequest {
    pub grant_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SharedNodeRequest {
    pub grant_id: String,
    pub node_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SharedQueryRequest {
    pub grant_id: String,
    pub node_id: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub sort: StorageSort,
    #[serde(default)]
    pub name_filter: Option<String>,
    #[serde(default)]
    pub thresholds: Option<HotspotThresholds>,
}
fn default_limit() -> usize {
    10
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SharedNodeResponse {
    pub schema_version: u32,
    pub grant_id: String,
    pub snapshot_id: String,
    pub revision: u64,
    pub node: StorageNode,
    pub reference: String,
    pub omitted_fields: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hotspot: Option<SharedHotspotContribution>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SharedHotspotContribution {
    pub reason: String,
    pub included_bytes: String,
    pub role: String,
    pub provisional: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SharedPageResponse {
    pub schema_version: u32,
    pub grant_id: String,
    pub snapshot_id: String,
    pub revision: u64,
    pub items: Vec<SharedNodeResponse>,
    pub total_count: Option<u64>,
    pub next_cursor: Option<String>,
    pub response_limited: bool,
    pub requested_limit: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolveEvidenceRequest {
    pub reference: String,
    pub thread_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct StorageEvidenceTarget {
    pub snapshot_id: String,
    pub revision: u64,
    pub node_id: String,
    pub ancestors: Vec<StorageNode>,
    pub node: StorageNode,
}

/// Identifiers only: the URI is a navigation hint, never a bearer authorization.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EvidenceReference {
    pub version: u32,
    pub host_id: String,
    pub thread_id: String,
    pub grant_id: String,
    pub snapshot_id: String,
    pub revision: u64,
    pub node_id: String,
}
