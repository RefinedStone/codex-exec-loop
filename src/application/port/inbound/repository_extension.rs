#[derive(serde::Serialize, schemars::JsonSchema)]
pub(crate) struct ExtensionManifest {
    pub id: &'static str,
    pub version: u32,
    pub scope: &'static str,
    pub enabled: bool,
    pub panel: &'static str,
    pub label: &'static str,
    pub actions: &'static [&'static str],
    pub permissions: &'static [&'static str],
}
