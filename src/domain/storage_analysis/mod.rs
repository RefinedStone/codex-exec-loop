//! Stable, lossless storage read model shared by Desktop, CLI and scoped AI queries.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod hotspots;
mod live;
pub use live::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageState {
    Pending,
    Running,
    Complete,
    Partial,
    Cancelled,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageNodeKind {
    Directory,
    File,
    DirectFiles,
    SharedFiles,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StorageIssue {
    pub code: String,
    pub message: String,
    pub count: u64,
}
impl StorageIssue {
    pub fn one(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            count: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StorageEvidence {
    pub code: String,
    pub detail: String,
    pub observed_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StorageClassification {
    pub kind: String,
    pub evidence: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StorageVolume {
    pub id: String,
    pub label: String,
    pub root_path: String,
    pub kind: String,
    pub supported: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StorageRatio {
    pub numerator_bytes: Option<String>,
    pub denominator_bytes: Option<String>,
    pub reason: Option<String>,
    pub provisional: bool,
}
impl StorageRatio {
    pub fn new(
        numerator: Option<u64>,
        denominator: Option<u64>,
        root: bool,
        provisional: bool,
    ) -> Self {
        let reason = if root {
            Some("root")
        } else if denominator == Some(0) {
            Some("zero_denominator")
        } else if denominator.is_none() || numerator.is_none() {
            Some("unknown")
        } else {
            None
        };
        Self {
            numerator_bytes: numerator.map(|v| v.to_string()),
            denominator_bytes: if root {
                None
            } else {
                denominator.map(|v| v.to_string())
            },
            reason: reason.map(str::to_owned),
            provisional,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StorageNode {
    pub snapshot_id: String,
    pub revision: u64,
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub kind: StorageNodeKind,
    /// None means the allocation is not fully known. Zero is a known zero.
    pub allocated_bytes: Option<String>,
    pub known_allocated_bytes: String,
    /// Aggregate logical bytes use the same unique-file owner as allocation; file queries retain every observed hardlink path.
    pub logical_bytes: String,
    pub ratio: StorageRatio,
    pub state: StorageState,
    pub issues: Vec<StorageIssue>,
    pub protection: Vec<StorageEvidence>,
    pub classification: Vec<StorageClassification>,
    pub allocation_owner_id: Option<String>,
}
impl StorageNode {
    pub fn known_bytes(&self) -> u64 {
        self.known_allocated_bytes.parse().unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StorageSnapshot {
    pub schema_version: u32,
    pub id: String,
    pub revision: u64,
    pub volume_id: String,
    pub root_node_id: String,
    pub state: StorageState,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub files_scanned: u64,
    pub directories_scanned: u64,
    pub issues: Vec<StorageIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ScanOptions {
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
}
const fn default_concurrency() -> usize {
    1
}
impl Default for ScanOptions {
    fn default() -> Self {
        Self { concurrency: 1 }
    }
}
impl ScanOptions {
    pub fn validate(&self) -> Result<(), String> {
        if (1..=4).contains(&self.concurrency) {
            Ok(())
        } else {
            Err("concurrency must be between 1 and 4".into())
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ScanRequest {
    pub job_id: String,
    pub volume_id: String,
    #[serde(default)]
    pub options: ScanOptions,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ScanProgress {
    pub volume_id: String,
    pub snapshot_id: String,
    pub files_scanned: u64,
    pub directories_scanned: u64,
    pub known_allocated_bytes: String,
    pub state: StorageState,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StorageSort {
    #[default]
    AllocatedDesc,
    Name,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StorageQuery {
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
    /// Server supplied grant root, never trusted directly from a model call.
    #[serde(default)]
    pub scope_root_id: Option<String>,
}
const fn default_limit() -> usize {
    10
}
impl StorageQuery {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=100).contains(&self.limit) {
            return Err("limit must be between 1 and 100".into());
        }
        if self.name_filter.as_ref().is_some_and(|s| s.len() > 256)
            || self.extension_filter.as_ref().is_some_and(|s| s.len() > 64)
        {
            return Err("filter is too long".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StoragePage<T> {
    pub snapshot_id: String,
    pub revision: u64,
    pub items: Vec<T>,
    pub total_count: Option<u64>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct HotspotThresholds {
    pub absolute_bytes: String,
    pub relative_min_bytes: String,
    pub parent_percent: u8,
}
impl Default for HotspotThresholds {
    fn default() -> Self {
        Self {
            absolute_bytes: (1024_u64.pow(3)).to_string(),
            relative_min_bytes: (256 * 1024_u64.pow(2)).to_string(),
            parent_percent: 20,
        }
    }
}
impl HotspotThresholds {
    pub fn values(&self) -> Result<(u64, u64, u8), String> {
        let a = self
            .absolute_bytes
            .parse::<u64>()
            .map_err(|_| "invalid absolute bytes")?;
        let r = self
            .relative_min_bytes
            .parse::<u64>()
            .map_err(|_| "invalid relative minimum")?;
        if a == 0 || r == 0 || !(1..=100).contains(&self.parent_percent) {
            return Err("thresholds must be positive and percent at most 100".into());
        }
        Ok((a, r, self.parent_percent))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StorageHotspot {
    pub included_bytes: String,
    pub provisional: bool,
    pub node: StorageNode,
    pub role: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lossless_bytes_and_ratio_states() {
        let ratio = StorageRatio::new(
            Some(9 * 1024_u64.pow(3)),
            Some(12 * 1024_u64.pow(3)),
            false,
            false,
        );
        assert_eq!(ratio.numerator_bytes.as_deref(), Some("9663676416"));
        assert_eq!(ratio.denominator_bytes.as_deref(), Some("12884901888"));
        assert!(ratio.reason.is_none());
        assert_eq!(
            StorageRatio::new(Some(0), Some(0), false, false)
                .reason
                .as_deref(),
            Some("zero_denominator")
        );
        assert_eq!(
            StorageRatio::new(None, Some(1), false, true)
                .reason
                .as_deref(),
            Some("unknown")
        );
        assert_eq!(
            StorageRatio::new(Some(1), None, true, false)
                .reason
                .as_deref(),
            Some("root")
        );
    }
}
