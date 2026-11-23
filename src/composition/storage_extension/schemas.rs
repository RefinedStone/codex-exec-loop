use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::application::service::storage_sharing::*;
use crate::domain::kernel_extension::{ExtensionOperation, ExtensionOperationKind};
use crate::domain::storage_analysis::*;

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Empty {}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ScanInput {
    pub volume_id: String,
    #[serde(default)]
    pub options: ScanOptions,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SnapshotInput {
    pub snapshot_id: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct NodeInput {
    pub snapshot_id: String,
    pub node_id: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct PriorityInput {
    pub job_id: String,
    pub node_id: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowseInput {
    pub snapshot_id: String,
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
    pub extension_filter: Option<String>,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub thresholds: Option<HotspotThresholds>,
}
fn default_limit() -> usize {
    10
}
impl BrowseInput {
    pub(super) fn query(&self) -> StorageQuery {
        StorageQuery {
            snapshot_id: self.snapshot_id.clone(),
            node_id: self.node_id.clone(),
            limit: self.limit,
            cursor: self.cursor.clone(),
            sort: self.sort.clone(),
            name_filter: self.name_filter.clone(),
            extension_filter: self.extension_filter.clone(),
            recursive: self.recursive,
            scope_root_id: None,
        }
    }
}
#[derive(Serialize, JsonSchema)]
struct Volumes {
    volumes: Vec<StorageVolume>,
}
#[derive(Serialize, JsonSchema)]
struct Ancestors {
    snapshot_id: String,
    node_id: String,
    ancestors: Vec<StorageNode>,
}
#[derive(Serialize, JsonSchema)]
struct RevealPath {
    path: String,
    host_id: String,
    snapshot_id: String,
    node_id: String,
}

fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schema_for!(T)).expect("static schema")
}
fn query<I: JsonSchema, O: JsonSchema>(name: &str) -> ExtensionOperation {
    ExtensionOperation {
        name: name.into(),
        kind: ExtensionOperationKind::Query,
        input_schema: schema::<I>(),
        output_schema: schema::<O>(),
        progress_schema: None,
    }
}
fn command<I: JsonSchema, O: JsonSchema>(name: &str) -> ExtensionOperation {
    ExtensionOperation {
        kind: ExtensionOperationKind::Command,
        ..query::<I, O>(name)
    }
}
fn hotspot_query<I: JsonSchema, O: JsonSchema>(name: &str) -> ExtensionOperation {
    let mut operation = query::<I, O>(name);
    let properties = operation.input_schema["properties"]
        .as_object_mut()
        .expect("query object schema");
    for (key, value) in [
        ("sort", json!({"const":"allocated_desc"})),
        ("name_filter", json!({"type":"null"})),
        ("extension_filter", json!({"type":"null"})),
        ("recursive", json!({"const":false})),
    ] {
        if properties.contains_key(key) {
            properties.insert(key.into(), value);
        }
    }
    operation
}
pub(super) fn operations() -> Vec<ExtensionOperation> {
    let mut scan = query::<ScanInput, StorageSnapshot>("scan");
    scan.kind = ExtensionOperationKind::Job;
    scan.progress_schema = Some(schema::<ScanProgress>());
    vec![
        query::<Empty, Volumes>("volumes"),
        scan,
        query::<StorageLiveQuery, StorageLivePage>("live_children"),
        command::<PriorityInput, StoragePriorityResult>("prioritize"),
        query::<SnapshotInput, StorageSnapshot>("snapshot"),
        query::<NodeInput, StorageNode>("get_node"),
        query::<BrowseInput, StoragePage<StorageNode>>("list_children"),
        query::<BrowseInput, StoragePage<StorageNode>>("list_files"),
        hotspot_query::<BrowseInput, StoragePage<StorageHotspot>>("find_hotspots"),
        query::<NodeInput, Ancestors>("ancestors"),
        query::<NodeInput, RevealPath>("reveal_path"),
        command::<SharePreviewRequest, StorageSharePreview>("share_preview"),
        command::<ShareCreateRequest, StorageShareGrant>("share_create"),
        command::<ShareRevokeRequest, Empty>("share_revoke"),
        query::<SharedNodeRequest, SharedNodeResponse>("shared_get_node"),
        query::<SharedQueryRequest, SharedPageResponse>("shared_list_children"),
        hotspot_query::<SharedQueryRequest, SharedPageResponse>("shared_find_hotspots"),
        query::<ResolveEvidenceRequest, StorageEvidenceTarget>("resolve_evidence"),
    ]
}
