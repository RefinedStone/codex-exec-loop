//! Mutable discovery metadata is deliberately separate from shareable, fixed snapshots.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StorageLiveQuery {
    pub job_id: String,
    pub node_id: String,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}
const fn default_limit() -> usize {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StorageLiveNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub enumeration_complete: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StorageLivePage {
    pub job_id: String,
    /// Zero denotes mutable discovery, never an AI evidence revision.
    pub revision: u64,
    pub root_node_id: String,
    pub node_id: String,
    pub items: Vec<StorageLiveNode>,
    pub total_count: Option<u64>,
    pub next_cursor: Option<String>,
    pub enumeration_complete: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StoragePriorityResult {
    pub job_id: String,
    pub node_id: String,
    pub queued: bool,
    pub queue_length: usize,
}
