use crate::domain::kernel_extension::{ExtensionCatalogEntry, ExtensionScope as HostScope};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtensionScope {
    pub kernel: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtensionCall {
    pub kernel: String,
    pub extension: String,
    pub version: u32,
    pub action: String,
    pub input: serde_json::Value,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtensionAttachRequest {
    pub scope: HostScope,
    pub client_name: String,
}
#[derive(serde::Serialize, JsonSchema)]
pub(crate) struct ExtensionAttachResponse {
    pub kernel: String,
    pub scope: HostScope,
    pub generation: String,
}
#[derive(serde::Serialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum ExtensionCatalogItem {
    Registered(ExtensionCatalogEntry),
    Legacy(crate::domain::repository_extension::ExtensionManifest),
}
#[derive(Default, serde::Serialize, JsonSchema)]
pub(crate) struct ExtensionChanged {
    pub kernel: String,
    pub extension: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resync: Option<bool>,
}
