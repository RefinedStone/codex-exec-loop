//! Read page revision metadata through the user's existing Notion MCP login.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::port::outbound::external_resource_version_port::ExternalResourceVersionPort;
use crate::application::port::outbound::mcp_hub_backend_port::McpHubBackendPort;
use crate::domain::work_context::ExternalResourceRef;

const SERVER: &str = "notion";

pub(crate) struct NotionResourceVersionAdapter {
    backend: Arc<dyn McpHubBackendPort>,
}

impl NotionResourceVersionAdapter {
    pub(crate) fn new(backend: Arc<dyn McpHubBackendPort>) -> Self {
        Self { backend }
    }
}

impl ExternalResourceVersionPort for NotionResourceVersionAdapter {
    fn kind(&self) -> &'static str {
        "notion"
    }

    fn available(&self, _resource: &ExternalResourceRef) -> bool {
        self.backend
            .registered_servers()
            .is_ok_and(|servers| servers.iter().any(|server| server.name == SERVER))
    }

    fn current_version(&self, key: &str) -> Result<String> {
        if key.len() != 32 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("invalid Notion page id")
        }
        if !self
            .backend
            .registered_servers()?
            .iter()
            .any(|server| server.name == SERVER)
        {
            bail!("Notion MCP is not registered")
        }
        let statuses = self.backend.server_statuses(Some(SERVER))?;
        let status = statuses
            .iter()
            .find(|status| status.name == SERVER)
            .context("Notion MCP returned no status")?;
        if status.error.is_some()
            || status.auth == "notLoggedIn"
            || status.connection == "authenticationRequired"
        {
            bail!("Notion MCP is unavailable")
        }
        let tool = ["notion-fetch", "fetch"]
            .into_iter()
            .find(|name| status.tools.iter().any(|tool| tool.name == *name))
            .context("Notion MCP has no page fetch tool")?;
        version_from_result(self.backend.call_tool(SERVER, tool, json!({"id": key}))?)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolResult {
    is_error: Option<bool>,
    structured_content: Option<Value>,
    #[serde(default)]
    content: Vec<ToolContent>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ToolContent {
    Text {
        text: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct PageVersion {
    page_last_edited_at: String,
}

fn version_from_result(result: Value) -> Result<String> {
    let result: ToolResult =
        serde_json::from_value(result).context("Notion MCP returned an invalid tool result")?;
    if result.is_error == Some(true) {
        bail!("Notion MCP could not read the page")
    }
    let page: PageVersion = match result.structured_content {
        Some(value) => {
            serde_json::from_value(value).context("Notion page has no edit timestamp")?
        }
        None => {
            let text = result
                .content
                .into_iter()
                .find_map(|content| match content {
                    ToolContent::Text { text } => Some(text),
                    ToolContent::Other => None,
                })
                .context("Notion MCP returned no page content")?;
            serde_json::from_str(&text).context("Notion page has no edit timestamp")?
        }
    };
    let timestamp = DateTime::parse_from_rfc3339(&page.page_last_edited_at)
        .context("Notion page returned an invalid edit timestamp")?;
    Ok(timestamp
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::AutoSi, true))
}

#[cfg(test)]
mod tests;
