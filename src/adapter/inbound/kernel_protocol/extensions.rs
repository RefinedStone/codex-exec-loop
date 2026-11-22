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
}

#[derive(serde::Serialize, JsonSchema)]
pub(crate) struct ExtensionChanged {
    pub kernel: String,
    pub extension: String,
}
