//! Versioned contracts for statically trusted kernel extensions.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
pub const EXTENSION_API_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionScope {
    Repository,
    LocalUser,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionOperationKind {
    Query,
    Command,
    Job,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtensionOperation {
    pub name: String,
    pub kind: ExtensionOperationKind,
    // Standalone document: local $defs never enter the built-in namespace.
    pub input_schema: Value,
    pub output_schema: Value,
    pub progress_schema: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtensionDescriptor {
    pub id: String,
    pub version: u32,
    pub api_version: u32,
    pub scope: ExtensionScope,
    pub label: String,
    pub panel: Option<String>,
    pub permissions: Vec<String>,
    pub operations: Vec<ExtensionOperation>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionLifecycle {
    Disabled,
    Enabled,
    Disabling,
    Error,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtensionActivation {
    pub desired_enabled: bool,
    pub lifecycle: ExtensionLifecycle,
    pub revision: u64,
    pub error: Option<String>,
}
impl Default for ExtensionActivation {
    fn default() -> Self {
        Self {
            desired_enabled: false,
            lifecycle: ExtensionLifecycle::Disabled,
            revision: 0,
            error: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ExtensionCatalogEntry {
    pub id: String,
    pub version: u32,
    pub api_version: u32,
    pub scope: ExtensionScope,
    pub label: String,
    /// Empty for a headless extension; a string for existing Desktop clients.
    pub panel: String,
    pub permissions: Vec<String>,
    pub actions: Vec<String>,
    pub operations: Vec<ExtensionOperation>,
    pub compatible: bool,
    pub compatibility_reason: Option<String>,
    pub enabled: bool,
    pub desired_enabled: bool,
    pub lifecycle: ExtensionLifecycle,
    pub revision: u64,
    pub generation: String,
    pub sequence: u64,
    pub registration_errors: Vec<String>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionJobState {
    Queued,
    Running,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}
impl ExtensionJobState {
    pub fn active(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Cancelling)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionCleanupState {
    Pending,
    Complete,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtensionJob {
    pub id: String,
    pub extension: String,
    pub action: String,
    pub state: ExtensionJobState,
    pub generation: String,
    pub revision: u64,
    pub sequence: u64,
    pub progress: Value,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub cleanup: ExtensionCleanupState,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub result_expires_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionReceipt {
    pub fingerprint: String,
    #[serde(default)]
    pub recorded_at_ms: u64,
    pub job_id: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtensionDurableState {
    pub host_id: String,
    pub activations: BTreeMap<String, ExtensionActivation>,
    pub jobs: BTreeMap<String, ExtensionJob>,
    pub receipts: BTreeMap<String, ExtensionReceipt>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ExtensionEvent {
    pub kernel: String,
    pub extension: String,
    pub generation: String,
    pub revision: u64,
    pub sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}
