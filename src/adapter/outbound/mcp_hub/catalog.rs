//! Search metadata is cached separately from live authentication and execution.

use std::collections::BTreeSet;

use anyhow::Result;
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, Item};

use super::tool_cache::{CatalogKey, ToolCatalogCache};
use super::{CodexMcpHubBackend, registered_servers_in, statuses_from};
use crate::domain::mcp_hub::McpHubToolCatalog;

impl CodexMcpHubBackend {
    pub(super) fn cached_tool_catalogs(&self) -> Result<Vec<McpHubToolCatalog>> {
        self.catalogs_at(chrono::Utc::now().timestamp(), |name, identity| {
            self.with_engine_for_catalog(Some(identity), |session, _| {
                statuses_from(session, Some(name))
            })
        })
    }

    fn catalogs_at(
        &self,
        now: i64,
        mut discover: impl FnMut(&str, &str) -> Result<Vec<crate::domain::mcp_hub::McpHubServerStatus>>,
    ) -> Result<Vec<McpHubToolCatalog>> {
        let document = self.read_document()?;
        let fingerprint = fingerprint(&document);
        let cache = ToolCatalogCache::new(&self.home);
        let generation = match cache.generation() {
            Ok(generation) => Some(generation),
            Err(error) => {
                tracing::warn!(error = %error, "MCP catalog cache is unavailable");
                None
            }
        };
        let identity = format!("{fingerprint}:{generation:?}");
        let mut catalogs = Vec::new();
        for server in registered_servers_in(&document) {
            if document
                .get("mcp_servers")
                .and_then(|servers| servers.get(&server.name))
                .and_then(|server| server.get("enabled"))
                .and_then(Item::as_bool)
                == Some(false)
            {
                continue;
            }
            let key = generation.map(|generation| CatalogKey {
                server: server.name.clone(),
                fingerprint: fingerprint.clone(),
                generation,
            });
            if let Some(key) = &key {
                match cache.lookup(key, now) {
                    Ok(Some(catalog)) => {
                        catalogs.push(catalog);
                        continue;
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!(error = %error, "MCP catalog cache read failed"),
                }
            }
            let statuses = discover(&server.name, &identity)?;
            let Some(status) = statuses
                .into_iter()
                .find(|status| status.name == server.name)
            else {
                continue;
            };
            let successful = status.error.is_none()
                && status.auth != "notLoggedIn"
                && matches!(
                    status.connection.as_str(),
                    "reachable" | "connected" | "ready"
                );
            let catalog = McpHubToolCatalog {
                name: status.name,
                tools: status.tools,
            };
            if successful && let Some(key) = &key {
                // A changed config cannot label a live response with its old identity.
                if fingerprint == self.current_catalog_fingerprint()?
                    && let Err(error) = cache.store(key, &catalog, now)
                {
                    tracing::warn!(error = %error, "MCP catalog cache write failed");
                }
            }
            catalogs.push(catalog);
        }
        Ok(catalogs)
    }

    fn current_catalog_fingerprint(&self) -> Result<String> {
        Ok(fingerprint(&self.read_document()?))
    }
}

fn fingerprint(document: &DocumentMut) -> String {
    fingerprint_with_environment(document, |name| {
        std::env::var_os(name).map(|value| value.to_string_lossy().into_owned())
    })
}

fn fingerprint_with_environment(
    document: &DocumentMut,
    mut environment: impl FnMut(&str) -> Option<String>,
) -> String {
    let mut names = BTreeSet::from([
        "PATH".to_string(),
        "HOME".to_string(),
        "USERPROFILE".to_string(),
        "HTTP_PROXY".to_string(),
        "HTTPS_PROXY".to_string(),
        "NO_PROXY".to_string(),
    ]);
    if let Some(servers) = document.get("mcp_servers").and_then(Item::as_table_like) {
        for (_, server) in servers.iter() {
            if let Some(name) = server.get("bearer_token_env_var").and_then(Item::as_str) {
                names.insert(name.to_owned());
            }
            if let Some(vars) = server.get("env_vars").and_then(Item::as_array) {
                names.extend(
                    vars.iter()
                        .filter_map(|name| name.as_str().map(str::to_owned)),
                );
            }
            if let Some(headers) = server.get("env_http_headers").and_then(Item::as_table_like) {
                names.extend(
                    headers
                        .iter()
                        .filter_map(|(_, value)| value.as_str().map(str::to_owned)),
                );
            }
        }
    }
    let mut digest = Sha256::new();
    digest.update(document.to_string().as_bytes());
    for name in names {
        digest.update([0]);
        digest.update(name.as_bytes());
        match environment(&name) {
            Some(value) => {
                digest.update([1]);
                digest.update(value.as_bytes());
            }
            None => digest.update([0]),
        }
    }
    format!("{:x}", digest.finalize())
}

#[cfg(test)]
mod tests;
